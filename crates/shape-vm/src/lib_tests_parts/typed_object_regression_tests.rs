#[cfg(test)]
mod typed_object_regression_tests {
    use crate::compiler::BytecodeCompiler;
    use crate::executor::{VMConfig, VirtualMachine};
    use shape_value::{ValueWord, ValueWordExt};

    /// Helper that compiles and executes a Shape snippet through the VM.
    fn eval(code: &str) -> ValueWord {
        let program = shape_ast::parser::parse_program(code).expect("parse failed");
        let compiler = BytecodeCompiler::new();

        let bytecode = compiler.compile(&program).expect("compile failed");
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        vm.populate_module_objects();
        vm.execute(None).expect("execution failed").clone()
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
            result.as_arc_string().expect("Expected String").as_ref() as &str,
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
            result
                .to_number()
                .expect("Numeric payload should be preserved through TypedObject match"),
            42.0
        );
    }
}

// =========================================================================
// Extension System Integration Tests (Phase 5)
// =========================================================================

