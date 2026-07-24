use super::*;

use crate::bytecode::CompiledAnnotation;
use shape_ast::ast::{Annotation, AnnotationDef, AnnotationHandlerType, Item};

fn parse(source: &str) -> shape_ast::ast::Program {
    shape_ast::parse_program(source).expect("helper-authority fixture parses")
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
    CompiledAnnotation {
        name: exact_name.to_string(),
        param_names: Vec::new(),
        param_defs: Vec::new(),
        on_define_handler: None,
        metadata_handler: None,
        comptime_pre_handler: def
            .handlers
            .iter()
            .find(|handler| handler.handler_type == AnnotationHandlerType::ComptimePre)
            .cloned(),
        comptime_post_handler: def
            .handlers
            .iter()
            .find(|handler| handler.handler_type == AnnotationHandlerType::ComptimePost)
            .cloned(),
        sugar_post_handler: None,
        sugar_body_fns: Vec::new(),
        allowed_targets: def.allowed_targets.clone().unwrap_or_default(),
    }
}

fn helper(source: &str, exact_name: &str) -> FunctionDef {
    let mut function = parse(source)
        .items
        .into_iter()
        .find_map(|item| match item {
            Item::Function(function, _) => Some(function),
            _ => None,
        })
        .expect("fixture defines one comptime helper");
    function.name = exact_name.to_string();
    function
}

fn install_annotation(compiler: &mut BytecodeCompiler, exact_name: &str, def: &AnnotationDef) {
    compiler
        .program
        .compiled_annotations
        .insert(exact_name.to_string(), compiled_annotation(exact_name, def));
}

fn install_helper(compiler: &mut BytecodeCompiler, exact_name: &str, value: i64) {
    let function = helper(
        &format!("comptime fn choose() -> int {{ {value} }}"),
        exact_name,
    );
    compiler
        .function_defs
        .insert(function.name.clone(), function);
}

fn selected_handler(target: &str) -> AnnotationDef {
    annotation_def(&format!(
        r#"
annotation mark() on {target} {{
  comptime post(target, ctx) {{
    if choose() == 11 {{
      error("SELECTED_DEFINING_MODULE")
    }} else {{
      error("WRONG_HELPER_SELECTED")
    }}
  }}
}}
"#
    ))
}

fn missing_handler(target: &str) -> AnnotationDef {
    annotation_def(&format!(
        r#"
annotation mark() on {target} {{
  comptime post(target, ctx) {{
    if choose() > 0 {{ error("BARE_HELPER_AUTHORITY_LEAK") }}
  }}
}}
"#
    ))
}

fn qualified_handler() -> AnnotationDef {
    annotation_def(
        r#"
annotation mark() on function {
  comptime post(target, ctx) {
    if other::choose() == 22 { error("EXPLICIT_QUALIFIED_HELPER") }
  }
}
"#,
    )
}

fn root_local_handler() -> AnnotationDef {
    annotation_def(
        r#"
annotation mark() on function {
  comptime post(target, ctx) {
    if choose() == 99 { error("ROOT_LOCAL_HELPER") }
  }
}
"#,
    )
}

fn applied(name: &str) -> Annotation {
    Annotation {
        name: name.to_string(),
        args: Vec::new(),
        span: Span::new(10, 20),
    }
}

fn target_function() -> FunctionDef {
    parse("fn probe() -> int { 0 }")
        .items
        .into_iter()
        .find_map(|item| match item {
            Item::Function(function, _) => Some(function),
            _ => None,
        })
        .expect("fixture defines one target function")
}

fn authoritative_execution(
    compiler: &mut BytecodeCompiler,
    annotation_name: &str,
    definition: &AnnotationDef,
) -> Result<crate::compiler::comptime::ComptimeExecutionResult> {
    let handler = definition
        .handlers
        .iter()
        .find(|handler| handler.handler_type == AnnotationHandlerType::ComptimePost)
        .expect("fixture has a comptime post handler");
    let target =
        crate::compiler::comptime_target::ComptimeTarget::from_function(&target_function())
            .to_nanboxed(None)?;
    // E1 slice-5: the handler executor takes the overlay explicitly now; the
    // fixture installs the freeze, so acquire it from the compiler.
    let overlay = compiler.comptime_freeze_overlay()?;
    compiler.execute_comptime_annotation_handler(
        &applied(annotation_name),
        handler,
        target,
        &[],
        &[],
        None,
        overlay,
    )
}

fn compiler_with_colliding_helpers(annotation: &AnnotationDef) -> BytecodeCompiler {
    let mut compiler = BytecodeCompiler::new();
    install_annotation(&mut compiler, "left::mark", annotation);
    install_helper(&mut compiler, "choose", 99);
    install_helper(&mut compiler, "left::choose", 11);
    install_helper(&mut compiler, "other::choose", 22);
    compiler
        .install_semantic_freeze()
        .expect("fixture installs the prepass freeze");
    compiler
}

#[test]
fn signature_consumer_uses_the_selected_annotations_defining_module_helper() {
    let mut compiler = compiler_with_colliding_helpers(&selected_handler("function"));
    let mut program = parse(
        r#"
@left::mark()
fn probe() -> int { 0 }
"#,
    );

    let error = compiler
        .apply_function_comptime_signature_directives_for_analysis(&mut program)
        .expect_err("selected defining-module helper reaches the handler");
    let message = error.to_string();
    assert!(
        message.contains("SELECTED_DEFINING_MODULE"),
        "got: {message}"
    );
    assert!(!message.contains("WRONG_HELPER_SELECTED"), "got: {message}");
}

#[test]
fn materialization_consumer_uses_the_selected_annotations_defining_module_helper() {
    let mut compiler = compiler_with_colliding_helpers(&selected_handler("type"));
    let program = parse(
        r#"
@left::mark()
type Probe { id: int }
"#,
    );

    let error = compiler
        .materialize_computed_comptime_extends(&program)
        .expect_err("selected defining-module helper reaches the handler");
    let message = error.to_string();
    assert!(
        message.contains("SELECTED_DEFINING_MODULE"),
        "got: {message}"
    );
    assert!(!message.contains("WRONG_HELPER_SELECTED"), "got: {message}");
}

#[test]
fn signature_consumer_does_not_fill_a_module_helper_miss_from_global_spelling() {
    let annotation = missing_handler("function");
    let mut compiler = compiler_with_colliding_helpers(&annotation);
    compiler.function_defs.remove("left::choose");
    let mut program = parse(
        r#"
@left::mark()
fn probe() -> int { 0 }
"#,
    );

    compiler
        .apply_function_comptime_signature_directives_for_analysis(&mut program)
        .expect("a defining-module miss defers without executing root or other-module helpers");
}

#[test]
fn materialization_consumer_does_not_fill_a_module_helper_miss_from_global_spelling() {
    let annotation = missing_handler("type");
    let mut compiler = compiler_with_colliding_helpers(&annotation);
    compiler.function_defs.remove("left::choose");
    let program = parse(
        r#"
@left::mark()
type Probe { id: int }
"#,
    );

    let generated = compiler
        .materialize_computed_comptime_extends(&program)
        .expect("a defining-module miss defers without executing root or other-module helpers");
    assert!(generated.is_empty());
}

#[test]
fn explicitly_qualified_helper_reference_is_preserved_without_a_bare_alias() {
    let mut compiler = compiler_with_colliding_helpers(&qualified_handler());
    compiler.function_defs.remove("left::choose");
    let mut program = parse(
        r#"
@left::mark()
fn probe() -> int { 0 }
"#,
    );

    let error = compiler
        .apply_function_comptime_signature_directives_for_analysis(&mut program)
        .expect_err("an explicit qualified reference remains available");
    assert!(
        error.to_string().contains("EXPLICIT_QUALIFIED_HELPER"),
        "got: {error}"
    );
}

#[test]
fn root_local_bare_helper_executes_through_explicit_root_authority() {
    let definition = root_local_handler();
    let mut compiler = BytecodeCompiler::new();
    install_helper(&mut compiler, "choose", 99);
    compiler
        .install_semantic_freeze()
        .expect("fixture installs the prepass freeze");
    let mut program = parse(
        r#"
@mark()
fn probe() -> int { 0 }
"#,
    );
    program
        .items
        .insert(0, Item::AnnotationDef(definition.clone(), definition.span));

    let error = compiler
        .apply_function_comptime_signature_directives_for_analysis(&mut program)
        .expect_err("the root-local helper must execute");
    assert!(
        error.to_string().contains("ROOT_LOCAL_HELPER"),
        "got: {error}"
    );
}

#[test]
fn authoritative_pass2_executes_the_exact_defining_module_helper() {
    let definition = selected_handler("function");
    let mut compiler = compiler_with_colliding_helpers(&definition);

    let error = match authoritative_execution(&mut compiler, "left::mark", &definition) {
        Ok(_) => panic!("the exact defining-module helper must execute in pass 2"),
        Err(error) => error,
    };
    let message = error.to_string();
    assert!(
        message.contains("SELECTED_DEFINING_MODULE"),
        "got: {message}"
    );
    assert!(!message.contains("WRONG_HELPER_SELECTED"), "got: {message}");
}

#[test]
fn authoritative_pass2_does_not_execute_root_on_a_defining_module_miss() {
    let definition = missing_handler("function");
    let mut compiler = compiler_with_colliding_helpers(&definition);
    compiler.function_defs.remove("left::choose");

    let error = match authoritative_execution(&mut compiler, "left::mark", &definition) {
        Ok(_) => panic!("a missing defining-module helper must remain unavailable in pass 2"),
        Err(error) => error,
    };
    let message = error.to_string();
    assert!(
        message.contains("choose"),
        "the refusal must name the missing helper: {message}"
    );
    assert!(
        !message.contains("BARE_HELPER_AUTHORITY_LEAK"),
        "the root helper executed: {message}"
    );
}
