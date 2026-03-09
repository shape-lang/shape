#[cfg(test)]
mod runtime_error_payload_tests {
    use super::*;
    use shape_runtime::engine::ShapeEngine;
    use shape_wire::WireValue;

    #[test]
    fn uncaught_match_exception_sets_structured_runtime_error_payload() {
        let mut engine = ShapeEngine::new().expect("engine should create");
        engine.load_stdlib().expect("stdlib should load");

        let mut executor = BytecodeExecutor::new();
        let source = "let c = \"s\"\nmatch c {\n  c: int => c\n}";

        let result = engine.execute(&mut executor, source);
        assert!(result.is_err(), "execution should fail");

        let payload = engine
            .runtime
            .take_last_runtime_error()
            .expect("runtime error payload should be present");

        match payload {
            WireValue::Object(obj) => {
                let category = obj.get("category").and_then(WireValue::as_str);
                assert_eq!(category, Some("AnyError"));
            }
            other => panic!("expected AnyError object payload, got {other:?}"),
        }
    }
}

