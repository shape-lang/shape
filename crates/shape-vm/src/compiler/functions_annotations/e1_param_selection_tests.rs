//! ADR-009 E1 #17 (slice 2, E1-D4) — ParamId selection + fail-closed C0930.
//!
//! The pre-E1 seam silently `continue`d when a `set param …` directive named a
//! parameter the frozen callable did not declare, dropping the directive. E1-D4
//! resolves the spelling to a position ONCE (`param_selection::resolve_param_id`)
//! and makes a miss the named hard error `[C0930]` (`ShapeError::SemanticError`).
//! These pins cover the seam directly (positive + both miss arms) and the known
//! imported-annotation-handler hazard end to end.

use super::*;

use crate::bytecode::CompiledAnnotation;
use crate::compiler::comptime_builtins::ComptimeDirective;
use shape_ast::ast::{AnnotationDef, AnnotationHandlerType, Item};

// --- direct seam pins ------------------------------------------------------

fn function_def(src: &str) -> FunctionDef {
    shape_ast::parse_program(src)
        .expect("fixture parses")
        .items
        .into_iter()
        .find_map(|item| match item {
            Item::Function(func, _) => Some(func),
            _ => None,
        })
        .expect("fixture defines one function")
}

fn loc() -> SourceLocation {
    BytecodeCompiler::new().span_to_source_location(Span::new(0, 0))
}

// POSITIVE: a directive naming a DECLARED parameter resolves to its position and
// the mutation applies.
#[test]
fn set_param_type_on_a_declared_param_resolves_and_applies() {
    let mut fd = function_def("fn f(real) -> int { 0 }");
    let directives = vec![ComptimeDirective::SetParamType {
        param_name: "real".to_string(),
        type_annotation: TypeAnnotation::Basic("int".to_string()),
    }];
    BytecodeCompiler::apply_signature_directives_to_analysis_function(
        &mut fd,
        directives,
        "mark",
        loc(),
    )
    .expect("a declared param resolves and the type applies");
    assert_eq!(
        fd.params[0].type_annotation,
        Some(TypeAnnotation::Basic("int".to_string()))
    );
}

// NEGATIVE (set param type): an UNDECLARED spelling is a hard [C0930] carrying
// directive kind + missing spelling + the actual param list, and mutates nothing.
#[test]
fn set_param_type_on_an_undeclared_param_is_c0930_not_a_silent_skip() {
    let mut fd = function_def("fn f(real) -> int { 0 }");
    let directives = vec![ComptimeDirective::SetParamType {
        param_name: "ghost".to_string(),
        type_annotation: TypeAnnotation::Basic("int".to_string()),
    }];
    let err = BytecodeCompiler::apply_signature_directives_to_analysis_function(
        &mut fd,
        directives,
        "mark",
        loc(),
    )
    .expect_err("an undeclared param spelling is a hard error, not a silent skip");
    let msg = err.to_string();
    assert!(msg.contains("[C0930]"), "got: {msg}");
    assert!(msg.contains("set param type"), "names the directive kind: {msg}");
    assert!(msg.contains("ghost"), "names the missing spelling: {msg}");
    assert!(msg.contains("real"), "lists the frozen callable's params: {msg}");
    // Fail-closed: the real param was left untouched.
    assert_eq!(fd.params[0].type_annotation, None);
}

// NEGATIVE (set param value): the SAME single resolution point covers the value
// arm — a miss is [C0930] before any value conversion runs.
#[test]
fn set_param_value_on_an_undeclared_param_is_c0930() {
    let mut fd = function_def("fn f(real) -> int { 0 }");
    let directives = vec![ComptimeDirective::SetParamValue {
        param_name: "ghost".to_string(),
        value: KindedSlot::from_int(0),
    }];
    let err = BytecodeCompiler::apply_signature_directives_to_analysis_function(
        &mut fd,
        directives,
        "mark",
        loc(),
    )
    .expect_err("an undeclared param spelling is a hard error for the value arm too");
    let msg = err.to_string();
    assert!(msg.contains("[C0930]"), "got: {msg}");
    assert!(msg.contains("set param value"), "names the directive kind: {msg}");
    assert!(msg.contains("ghost"), "names the missing spelling: {msg}");
}

// PASS-2 UNIFICATION (E1-D4 resolve-ONCE): the install-phase applier
// (`process_comptime_directives_for_function`) now resolves through the SAME
// `resolve_param_id`, called with no annotation/span context (`None, None`).
// That call shape is normally preempted by the analysis-pre-pass [C0930] on a
// full compile, so it is pinned here directly against the unified helper: still
// [C0930], just without the `from @…` clause. This is the deleted divergent
// "referenced unknown parameter" message's replacement — one diagnostic, one
// resolver.
#[test]
fn resolve_param_id_without_annotation_context_is_still_c0930() {
    let fd = function_def("fn f(real) -> int { 0 }");
    let err = super::param_selection::resolve_param_id(&fd, "ghost", "set param type", None, None)
        .expect_err("a miss is [C0930] even without annotation/span context");
    let msg = err.to_string();
    assert!(msg.contains("[C0930]"), "got: {msg}");
    assert!(
        !msg.contains("from @"),
        "no annotation clause when context is absent: {msg}"
    );
    assert!(msg.contains("ghost"), "names the missing spelling: {msg}");
    assert!(msg.contains("real"), "lists the frozen callable's params: {msg}");
}

// --- imported-annotation-handler hazard (end to end) -----------------------
//
// The known hazard (C2 finding 3): imported-annotation-handler outcomes were at
// risk of vanishing. Here an IMPORTED (compiled) handler successfully emits a
// `set param` naming a parameter the target does not declare; slice 2 makes that
// miss surface as [C0930] through the full signature-directive pass, not vanish.
// Harness mirrors `imported_handler_resolution_tests`.

fn parse(source: &str) -> shape_ast::ast::Program {
    shape_ast::parse_program(source).expect("hazard fixture parses")
}

fn annotation_def(source: &str) -> AnnotationDef {
    parse(source)
        .items
        .into_iter()
        .find_map(|item| match item {
            Item::AnnotationDef(def, _) => Some(def),
            _ => None,
        })
        .expect("fixture defines one annotation")
}

fn compiled_annotation(exact_name: &str, def: &AnnotationDef) -> CompiledAnnotation {
    let comptime_pre_handler = def
        .handlers
        .iter()
        .find(|handler| handler.handler_type == AnnotationHandlerType::ComptimePre)
        .cloned();
    let comptime_post_handler = def
        .handlers
        .iter()
        .find(|handler| handler.handler_type == AnnotationHandlerType::ComptimePost)
        .cloned();
    CompiledAnnotation {
        name: exact_name.to_string(),
        param_names: def
            .params
            .iter()
            .flat_map(|param| param.get_identifiers())
            .collect(),
        param_defs: def.params.clone(),
        before_handler: None,
        after_handler: None,
        on_define_handler: None,
        metadata_handler: None,
        comptime_pre_handler,
        comptime_post_handler,
        before_handler_template: None,
        after_handler_template: None,
        allowed_targets: def.allowed_targets.clone().unwrap_or_default(),
    }
}

fn install_compiled_annotation(
    compiler: &mut BytecodeCompiler,
    key: &str,
    carrier_name: &str,
    def: &AnnotationDef,
) {
    compiler
        .program
        .compiled_annotations
        .insert(key.to_string(), compiled_annotation(carrier_name, def));
}

#[test]
fn imported_handler_param_miss_surfaces_c0930_not_vanishes() {
    // An imported handler whose directive names a parameter the target does not
    // declare. The handler itself SUCCEEDS (it just emits the directive); the
    // miss is caught at the shared param-selection seam.
    let imported = annotation_def(
        r#"
annotation mark() {
  targets: [function]
  comptime post(target, ctx) {
    set param ghost: int
  }
}
"#,
    );
    let mut program = parse(
        r#"
@remote::mark()
fn probe(real: int) -> int { 0 }
"#,
    );
    let mut compiler = BytecodeCompiler::new();
    install_compiled_annotation(&mut compiler, "remote::mark", "remote::mark", &imported);
    compiler
        .install_semantic_freeze()
        .expect("fixture installs the prepass freeze");

    let error = compiler
        .apply_function_comptime_signature_directives_for_analysis(&mut program)
        .expect_err("an imported handler's param-miss directive must surface C0930, not vanish");
    let msg = error.to_string();
    assert!(msg.contains("[C0930]"), "imported-handler miss must be C0930: {msg}");
    assert!(msg.contains("ghost"), "names the missing spelling: {msg}");
    assert!(msg.contains("real"), "lists the frozen callable's params: {msg}");
}
