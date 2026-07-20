use super::handler_resolution::ComptimeAnnotationHandlerProvenance;
use super::*;

use crate::bytecode::CompiledAnnotation;
use shape_ast::ast::{Annotation, AnnotationDef, AnnotationHandlerType, Item};

fn parse(source: &str) -> shape_ast::ast::Program {
    shape_ast::parse_program(source).expect("handler-resolution fixture parses")
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

fn applied(name: &str) -> Annotation {
    Annotation {
        name: name.to_string(),
        args: Vec::new(),
        span: Span::new(10, 20),
    }
}

fn error_handler(marker: &str, target: &str) -> AnnotationDef {
    annotation_def(&format!(
        r#"
annotation mark() {{
  targets: [{target}]
  comptime post(target, ctx) {{ error("{marker}") }}
}}
"#
    ))
}

#[test]
fn handler_rows_use_recursive_local_paths_and_exact_compiled_keys() {
    let program = parse(
        r#"
annotation mark() {
  comptime post(target, ctx) { 0 }
}
mod outer {
  annotation mark() {
    comptime post(target, ctx) { 1 }
  }
  mod inner {
    annotation mark() {
      comptime post(target, ctx) { 2 }
    }
  }
}
"#,
    );
    let imported = annotation_def(
        r#"
annotation mark(policy) {
  targets: [function]
  comptime post(target, ctx) { error("IMPORTED") }
}
"#,
    );
    let mut compiler = BytecodeCompiler::new();
    install_compiled_annotation(
        &mut compiler,
        "pkg::support::mark",
        "pkg::support::mark",
        &imported,
    );

    let rows = compiler
        .collect_comptime_annotation_handlers(&program)
        .expect("well-formed rows collect");

    for key in ["mark", "outer::mark", "outer::inner::mark"] {
        assert_eq!(
            rows[key].provenance,
            ComptimeAnnotationHandlerProvenance::LocalAst,
            "{key} must retain local-AST authority"
        );
    }
    assert_eq!(rows["mark"].defining_module_path, None);
    assert_eq!(
        rows["outer::mark"].defining_module_path.as_deref(),
        Some("outer")
    );
    assert_eq!(
        rows["outer::inner::mark"].defining_module_path.as_deref(),
        Some("outer::inner")
    );
    assert_eq!(
        rows["pkg::support::mark"].provenance,
        ComptimeAnnotationHandlerProvenance::Compiled
    );
    assert_eq!(
        rows["pkg::support::mark"].defining_module_path.as_deref(),
        Some("pkg::support")
    );
    assert_eq!(
        rows["pkg::support::mark"].def_params,
        vec![("policy".to_string(), None)]
    );
}

#[test]
fn resolution_is_exact_and_local_ast_is_the_only_prepass_fallback() {
    let program = parse(
        r#"
annotation mark() { comptime post(target, ctx) { 0 } }
mod outer {
  annotation mark() { comptime post(target, ctx) { 1 } }
}
"#,
    );
    let imported = error_handler("REMOTE", "function");
    let mut compiler = BytecodeCompiler::new();
    install_compiled_annotation(&mut compiler, "remote::mark", "remote::mark", &imported);
    let rows = compiler
        .collect_comptime_annotation_handlers(&program)
        .expect("well-formed rows collect");

    let (root_key, root) = compiler
        .resolve_comptime_annotation_handlers(&rows, &applied("mark"), None)
        .expect("bare root application resolves to root LocalAst row");
    assert_eq!(root_key, "mark");
    assert_eq!(
        root.provenance,
        ComptimeAnnotationHandlerProvenance::LocalAst
    );

    let (local_key, local) = compiler
        .resolve_comptime_annotation_handlers(&rows, &applied("mark"), Some("outer"))
        .expect("bare inline application resolves by exact lexical path");
    assert_eq!(local_key, "outer::mark");
    assert_eq!(
        local.provenance,
        ComptimeAnnotationHandlerProvenance::LocalAst
    );

    let (remote_key, remote) = compiler
        .resolve_comptime_annotation_handlers(&rows, &applied("remote::mark"), None)
        .expect("qualified compiled application resolves exactly");
    assert_eq!(remote_key, "remote::mark");
    assert_eq!(
        remote.provenance,
        ComptimeAnnotationHandlerProvenance::Compiled
    );

    assert!(
        compiler
            .resolve_comptime_annotation_handlers(&rows, &applied("ghost::mark"), None)
            .is_none(),
        "a qualified miss must not fall back to a same-spelled bare row"
    );
    assert!(
        compiler
            .resolve_comptime_annotation_handlers(&rows, &applied("mark"), Some("ghost"))
            .is_none(),
        "a lexical miss must not scan other local modules or the root row"
    );
}

#[test]
fn malformed_compiled_registry_key_is_rejected_before_row_publication() {
    let imported = error_handler("BROKEN", "function");
    let mut compiler = BytecodeCompiler::new();
    install_compiled_annotation(&mut compiler, "pkg::declared", "pkg::different", &imported);

    let error = compiler
        .collect_comptime_annotation_handlers(&parse(""))
        .expect_err("key/name drift must be rejected");
    assert_eq!(
        error.to_string(),
        "Runtime error: Internal error: compiled annotation registry key \
         'pkg::declared' does not match carrier name 'pkg::different'"
    );
}

#[test]
fn discovery_consumer_selects_only_the_exact_qualified_compiled_handler() {
    let left = error_handler("LEFT_DISCOVERY", "type");
    let right = error_handler("RIGHT_DISCOVERY", "type");
    let program = parse(
        r#"
@left::mark()
type Probe { id: int }
"#,
    );
    let mut compiler = BytecodeCompiler::new();
    install_compiled_annotation(&mut compiler, "left::mark", "left::mark", &left);
    install_compiled_annotation(&mut compiler, "right::mark", "right::mark", &right);
    compiler
        .install_semantic_freeze()
        .expect("fixture installs the prepass freeze");

    let error = compiler
        .materialize_computed_comptime_extends(&program)
        .expect_err("the selected handler's error must surface");
    let message = error.to_string();
    assert!(message.contains("LEFT_DISCOVERY"), "got: {message}");
    assert!(!message.contains("RIGHT_DISCOVERY"), "got: {message}");
}

#[test]
fn signature_consumer_selects_only_the_exact_qualified_compiled_handler() {
    let left = error_handler("LEFT_SIGNATURE", "function");
    let right = error_handler("RIGHT_SIGNATURE", "function");
    let mut program = parse(
        r#"
@right::mark()
fn probe() -> int { 0 }
"#,
    );
    let mut compiler = BytecodeCompiler::new();
    install_compiled_annotation(&mut compiler, "left::mark", "left::mark", &left);
    install_compiled_annotation(&mut compiler, "right::mark", "right::mark", &right);
    compiler
        .install_semantic_freeze()
        .expect("fixture installs the prepass freeze");

    let error = compiler
        .apply_function_comptime_signature_directives_for_analysis(&mut program)
        .expect_err("the selected handler's error must surface");
    let message = error.to_string();
    assert!(message.contains("RIGHT_SIGNATURE"), "got: {message}");
    assert!(!message.contains("LEFT_SIGNATURE"), "got: {message}");
}

#[test]
fn inline_module_signature_traversal_uses_and_restores_lexical_scope() {
    let mut program = parse(
        r#"
mod nested {
  annotation mark() {
    targets: [function]
    comptime post(target, ctx) { error("INLINE_LOCAL") }
  }
  @mark()
  fn probe() -> int { 0 }
}
"#,
    );
    let mut compiler = BytecodeCompiler::new();
    compiler
        .install_semantic_freeze()
        .expect("fixture installs the prepass freeze");

    let error = compiler
        .apply_function_comptime_signature_directives_for_analysis(&mut program)
        .expect_err("the inline-local handler must be selected");
    assert!(error.to_string().contains("INLINE_LOCAL"), "got: {error}");
    assert!(
        compiler.module_scope_stack.is_empty(),
        "lexical module scope must be restored on the error path"
    );
}
