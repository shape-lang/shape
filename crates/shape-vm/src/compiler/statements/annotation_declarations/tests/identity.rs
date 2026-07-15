use super::*;

#[test]
fn lifecycle_handler_id_survives_nested_closure_registration() {
    let program = parse(
        r#"
annotation nested() {
  metadata(target) {
    let identity = |value| value
    identity(target)
  }
}
"#,
    );
    let mut compiler = BytecodeCompiler::new();
    compiler
        .prepare_annotation_scope(&program.items)
        .expect("metadata handler with closure installs");
    let carrier = compiler
        .program
        .compiled_annotations
        .get("nested")
        .expect("carrier published");
    let handler_id = usize::from(carrier.metadata_handler.expect("handler id"));
    assert_eq!(
        compiler.program.functions[handler_id].name,
        "nested___metadata"
    );
    assert!(
        compiler
            .program
            .functions
            .iter()
            .skip(handler_id + 1)
            .any(|function| function.is_closure),
        "closure rows may follow the reserved handler row without stealing its id"
    );
    let hash = compiler.function_hashes_by_id[handler_id].expect("handler hash");
    assert_eq!(compiler.blob_name_to_hash.get("nested___metadata"), Some(&hash));
}

#[test]
fn bare_and_qualified_callable_collisions_refuse_before_installation() {
    for module_path in [None, Some("pkg")] {
        let mut compiler = BytecodeCompiler::new();
        let occupied = parse("fn blocked___metadata() -> int { 1 }");
        let Item::Function(mut function, _) = occupied.items[0].clone() else {
            panic!("fixture function")
        };
        let callable_name = module_path
            .map(|module| format!("{}::blocked___metadata", module))
            .unwrap_or_else(|| "blocked___metadata".to_string());
        function.name = callable_name.clone();
        compiler
            .register_function(&function)
            .expect("occupy callable name");
        let definition = parse("annotation blocked() { metadata(target) { 1 } }");
        let items = if let Some(module) = module_path {
            vec![compiler
                .qualify_module_item(&definition.items[0], module)
                .expect("definition qualifies")]
        } else {
            vec![definition.items[0].clone()]
        };
        let before = compiler.program.functions.len();
        let error = compiler
            .prepare_annotation_scope(&items)
            .expect_err("planner must reject every occupied exact callable name");
        assert!(error.to_string().contains(&callable_name));
        assert_eq!(compiler.program.functions.len(), before);
        assert!(compiler.program.compiled_annotations.is_empty());
    }
}
