// Tests gated `deep-tests` post-W11: bodies depend on the deleted
// `ValueWord` / `ValueWordExt` ABI. Restoration requires migration of
// these regression tests to the kinded `KindedSlot` API per ADR-006
// §2.7.4 Phase-2c.
#[cfg(all(test, feature = "deep-tests"))]
mod typed_object_regression_tests {
    use crate::compiler::BytecodeCompiler;
    use crate::executor::{VMConfig, VirtualMachine};
    use shape_value::KindedSlot;

    /// Helper that compiles and executes a Shape snippet through the VM.
    fn eval(code: &str) -> KindedSlot {
        // Per-test TypeSchemaRegistry scope; see module_qualified_type_tests
        // for the rationale. Without this, concurrent compiles of enum
        // payload types with overlapping field layouts can observe each
        // other's SchemaIds via FALLBACK_PREDECLARED_REGISTRY, causing
        // GetFieldTyped to read stale slots and the EqInt slow path to
        // call as_i64_unchecked on a TAG_NONE ValueWord.
        let _schema_scope = shape_runtime::type_schema::SyncRegistryScope::enter(
            std::sync::Arc::new(shape_runtime::type_schema::TypeSchemaRegistry::new_with_stdlib()),
        );

        let program = shape_ast::parser::parse_program(code).expect("parse failed");
        let compiler = BytecodeCompiler::new();

        let bytecode = compiler.compile(&program).expect("compile failed");
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        vm.populate_module_objects();
        vm.execute(None).expect("execution failed")
    }

    fn as_test_number(slot: &KindedSlot) -> Option<f64> {
        slot.as_f64().or_else(|| slot.as_i64().map(|i| i as f64))
    }

    /// Enum with a string payload must preserve the string through TypedObject slots.
    /// Before the ValueSlot fix, string payloads were silently lost (stored as 0u64).
    #[test]
    fn test_enum_string_payload_preserved() {
        let result = eval(
            r#"
            enum Message { Text(string), Empty }
            let m = Message::Text("hello")
            match m {
                Message::Text(s) => s,
                Message::Empty => "empty",
            }
        "#,
        );
        assert_eq!(
            result.as_str().expect("Expected String"),
            "hello",
            "String payload should be preserved through TypedObject match"
        );
    }

    /// Enum with a numeric payload must preserve the number through TypedObject slots.
    #[test]
    fn test_enum_number_payload_preserved() {
        let result = eval(
            r#"
            enum Outcome { Ok(number), Err(string) }
            let r = Outcome::Ok(42)
            match r {
                Outcome::Ok(n) => n,
                Outcome::Err(s) => 0,
            }
        "#,
        );
        assert_eq!(
            as_test_number(&result)
                .expect("Numeric payload should be preserved through TypedObject match"),
            42.0
        );
    }
}

// =========================================================================
// Extension System Integration Tests (Phase 5)
// =========================================================================
