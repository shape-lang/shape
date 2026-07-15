use super::*;

use shape_ast::ast::{CaptureMode, DestructurePattern, Span};

use crate::compiler::comptime_builtins::capture_plan::CaptureAccess;
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
    fn annotation_names(self) -> &'static [&'static str] {
        match self {
            Self::SingleRuntime => &["first"],
            Self::ChainedRuntime => &["first", "second"],
            Self::ReplaceBody => &["replace_with_original"],
        }
    }

    fn source(self, parameter: &str) -> String {
        let source = match self {
            Self::SingleRuntime => {
                r#"
annotation first() {
  targets: [function]
  comptime pre(target, ctx) { set param value: int }
  before(args, ctx) { args }
}

@first()
fn probe($PARAM) -> int {
  let worker = |x: int; move value| x + value
  worker(1)
}
probe(2)
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
    for name in route.annotation_names() {
        assert!(
            compiler.program.compiled_annotations.contains_key(*name),
            "fixture annotation '{name}' must be registered before probe compilation"
        );
    }

    let root = compiler.generated_node_issuer.issue(
        shape_ast::ast::GeneratedExpansionFingerprint::from_components(17, 29),
        shape_ast::ast::GeneratedNodePath::decl_root("fn:probe"),
        0,
        Span::DUMMY,
        "probe".to_string(),
    );
    let is_untyped_single =
        matches!(route, AnnotationRoute::SingleRuntime) && parameter == "value";
    let probe = program
        .items
        .iter_mut()
        .find_map(|item| match item {
            Item::Function(function, _) if function.name == "probe" => Some(function),
            _ => None,
        })
        .expect("fixture has probe");
    if is_untyped_single {
        let original_param = probe.params.first().expect("probe has one parameter");
        assert!(
            original_param.type_annotation.is_none(),
            "single-runtime source parameter must remain untyped"
        );
        assert_eq!(
            (
                original_param.is_reference,
                original_param.is_mut_reference
            ),
            (false, false),
            "single-runtime source parameter must not declare reference provenance"
        );
    }
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
    parameter: &str,
    mode: ParamPassMode,
) {
    let (compiler, outcome) = compile_stamped_probe(route, parameter, mode);
    outcome.expect("an inferred reference optimization is not a true reference capture");

    let descriptors: Vec<_> = compiler
        .closure_capture_packs
        .iter()
        .flat_map(|pack| &pack.descriptors)
        .filter(|descriptor| descriptor.name == "value")
        .collect();
    assert_eq!(descriptors.len(), 1, "fixture has one value capture");
    assert_eq!(descriptors[0].declared, Some(CaptureMode::Move));
    assert_eq!(
        descriptors[0].storage,
        Some(BindingStorageClass::LocalMutablePtr),
        "inferred pass mode must preserve exact stack-resident non-reference storage evidence"
    );
    assert_eq!(
        descriptors[0].access,
        CaptureAccess::Param,
        "inferred pass mode must preserve exact by-value capture access"
    );
    assert_route_artifacts(&compiler, route);
}

fn stamped_capture_body(function: &FunctionDef) -> bool {
    function.body.iter().any(|statement| {
        matches!(
            statement,
            Statement::VariableDecl(
                shape_ast::ast::VariableDecl {
                    value: Some(Expr::FunctionExpr {
                        generated_origin: Some(_),
                        ..
                    }),
                    ..
                },
                _
            )
        )
    })
}

fn registered_calls(compiler: &BytecodeCompiler, function_name: &str) -> Vec<String> {
    let function = compiler
        .program
        .functions
        .iter()
        .find(|function| function.name == function_name)
        .unwrap_or_else(|| panic!("registered function '{function_name}' is missing"));
    let end =
        (function.entry_point + function.body_length).min(compiler.program.instructions.len());
    compiler.program.instructions[function.entry_point..end]
        .iter()
        .filter_map(|instruction| match instruction.operand {
            Some(Operand::Function(id)) if instruction.opcode == OpCode::Call => compiler
                .program
                .functions
                .get(id.index())
                .map(|called| called.name.clone()),
            _ => None,
        })
        .collect()
}

fn generated_body_function(compiler: &BytecodeCompiler) -> String {
    let candidates: Vec<_> = compiler
        .function_defs
        .iter()
        .filter(|(name, function)| name.starts_with('\u{1}') && stamped_capture_body(function))
        .map(|(name, _)| name.clone())
        .collect();
    assert_eq!(
        candidates.len(),
        1,
        "exactly one registered hygienic function must own the stamped capture body"
    );
    let name = candidates[0].clone();
    assert!(
        compiler
            .program
            .functions
            .iter()
            .any(|function| function.name == name),
        "stamped hygienic body must exist in the bytecode function registry"
    );
    name
}

fn assert_route_artifacts(compiler: &BytecodeCompiler, route: AnnotationRoute) {
    assert!(
        compiler.function_defs.contains_key("probe")
            && compiler
                .program
                .functions
                .iter()
                .any(|function| function.name == "probe"),
        "the public probe function must remain registered"
    );
    let generated_body = generated_body_function(compiler);
    match route {
        AnnotationRoute::SingleRuntime => {
            assert_eq!(
                registered_calls(compiler, "probe")
                    .iter()
                    .filter(|name| *name == &generated_body)
                    .count(),
                1,
                "the public single-annotation wrapper must call its hygienic impl body exactly once"
            );
        }
        AnnotationRoute::ChainedRuntime => {
            let intermediates: Vec<_> = registered_calls(compiler, "probe")
                .into_iter()
                .filter(|name| {
                    name.starts_with('\u{1}')
                        && registered_calls(compiler, name).contains(&generated_body)
                })
                .collect();
            assert_eq!(
                intermediates.len(),
                1,
                "the public chained wrapper must call one registered hygienic wrapper that calls the impl body"
            );
            assert!(
                compiler
                    .function_defs
                    .get(&intermediates[0])
                    .is_some_and(|function| function.body.is_empty()),
                "the intermediate chain node must be the compiler's empty-body wrapper placeholder"
            );
            assert_eq!(
                registered_calls(compiler, &intermediates[0])
                    .iter()
                    .filter(|name| *name == &generated_body)
                    .count(),
                1,
                "the intermediate wrapper must call the hygienic impl body exactly once"
            );
        }
        AnnotationRoute::ReplaceBody => {
            let refreshed = compiler
                .function_defs
                .get("probe")
                .expect("replace body refreshes the registered probe definition");
            let shadow = match refreshed.body.as_slice() {
                [Statement::Return(Some(Expr::FunctionCall { name, .. }), _)] => name,
                body => panic!(
                    "refreshed probe must directly call its original-body shadow, got {body:?}"
                ),
            };
            assert!(shadow.starts_with('\u{1}'), "shadow identity must be hygienic");
            assert_ne!(shadow, "worker", "replacement must not call the local closure");
            assert_eq!(shadow, &generated_body);
            assert!(compiler.function_defs.contains_key(shadow));
            assert!(
                compiler
                    .program
                    .functions
                    .iter()
                    .any(|function| &function.name == shadow),
                "original-body shadow must be registered in bytecode"
            );
            assert_eq!(
                registered_calls(compiler, "probe")
                    .iter()
                    .filter(|name| *name == shadow)
                    .count(),
                1,
                "replacement bytecode must directly call the same registered original-body shadow"
            );
        }
    }
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
        "value",
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
        "value: int",
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
        "value: int",
        ParamPassMode::ByRefExclusive,
    );
    assert_explicit_reference_is_c0902(
        AnnotationRoute::ReplaceBody,
        "&mut value: int",
        ParamPassMode::ByRefExclusive,
    );
}
