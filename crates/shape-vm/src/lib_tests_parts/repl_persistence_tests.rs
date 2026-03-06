#[cfg(test)]
mod repl_persistence_tests {
    use super::*;
    use shape_runtime::engine::{ProgramExecutor, ShapeEngine};
    use shape_wire::WireValue;

    /// Helper to run REPL-style execution (mimics what execute_repl does)
    fn execute_repl_command(
        engine: &mut ShapeEngine,
        source: &str,
    ) -> shape_runtime::error::Result<WireValue> {
        let program = shape_ast::parser::parse_program(source)?;

        // Use incremental analysis for REPL (persistent state)
        engine.analyze_incremental(&program, source)?;

        // Execute via VM
        let executor = BytecodeExecutor::new();
        let result = executor.execute_program(engine, &program)?;
        Ok(result.wire_value)
    }

    /// Test that variables persist between separate VM executions via ExecutionContext
    #[test]
    fn test_variable_persistence_across_executions() {
        // Create an engine with persistent context
        let mut engine = ShapeEngine::new().expect("engine should create");
        engine.load_stdlib().expect("stdlib should load");
        engine.init_repl(); // Initialize REPL scope

        // First execution: define a variable
        let result1 = execute_repl_command(&mut engine, "let a = 42");
        assert!(
            result1.is_ok(),
            "first execution should succeed: {:?}",
            result1
        );

        // Second execution: use the variable
        let result2 = execute_repl_command(&mut engine, "a");
        assert!(
            result2.is_ok(),
            "second execution should succeed: {:?}",
            result2
        );

        let wire_val = result2.unwrap();
        assert_eq!(
            wire_val.as_number(),
            Some(42.0),
            "variable 'a' should be 42"
        );
    }

    /// Test that variables can be updated across executions
    #[test]
    fn test_variable_update_persistence() {
        let mut engine = ShapeEngine::new().expect("engine should create");
        engine.load_stdlib().expect("stdlib should load");
        engine.init_repl();

        // First: define variable
        execute_repl_command(&mut engine, "let x = 10").expect("should execute");

        // Second: update variable
        execute_repl_command(&mut engine, "x = 20").expect("should execute");

        // Third: read updated value
        let wire_val = execute_repl_command(&mut engine, "x").expect("should execute");
        assert_eq!(
            wire_val.as_number(),
            Some(20.0),
            "variable 'x' should be updated to 20"
        );
    }

    /// Test variable persistence with BytecodeExecutor (matches notebook executor)
    #[test]
    fn test_variable_persistence_with_stdlib_executor() {
        let mut engine = ShapeEngine::new().expect("engine should create");
        engine.load_stdlib().expect("stdlib should load");
        engine.init_repl();

        let executor = BytecodeExecutor::new();

        // Cell 1: define variable
        let program1 = shape_ast::parser::parse_program("let x = 42").expect("parse");
        engine
            .analyze_incremental(&program1, "let x = 42")
            .expect("analyze");
        let result1 = executor.execute_program(&mut engine, &program1);
        assert!(
            result1.is_ok(),
            "cell 1 should succeed: {:?}",
            result1.err()
        );

        // Cell 2: use variable from cell 1
        let program2 = shape_ast::parser::parse_program("x + 8").expect("parse");
        engine
            .analyze_incremental(&program2, "x + 8")
            .expect("analyze");
        let result2 = executor.execute_program(&mut engine, &program2);
        assert!(
            result2.is_ok(),
            "cell 2 should succeed: {:?}",
            result2.err()
        );

        let wire_val = result2.unwrap().wire_value;
        assert_eq!(
            wire_val.as_number(),
            Some(50.0),
            "x + 8 should be 50"
        );
    }

    /// Test multiple variables persist
    #[test]
    fn test_multiple_variables_persist() {
        let mut engine = ShapeEngine::new().expect("engine should create");
        engine.load_stdlib().expect("stdlib should load");
        engine.init_repl();

        // Define multiple variables
        execute_repl_command(&mut engine, "let a = 1").expect("should execute");
        execute_repl_command(&mut engine, "let b = 2").expect("should execute");

        // Use both variables
        let wire_val = execute_repl_command(&mut engine, "a + b").expect("should execute");
        assert_eq!(wire_val.as_number(), Some(3.0), "a + b should be 3");
    }

    /// Verifies no module binding index misalignment after merge_prepend elimination.
    /// The prelude now inlines stdlib definitions via AST inlining,
    /// so module binding indices are assigned in a single compilation pass.
    #[test]
    fn test_repl_with_stdlib_constants() {
        let mut engine = ShapeEngine::new().expect("engine should create");
        engine.load_stdlib().expect("stdlib should load");
        engine.init_repl();

        // Cell 1: Use stdlib function (abs is a prelude-injected builtin)
        let result1 = execute_repl_command(&mut engine, "let x = abs(-42)\nx");
        assert!(
            result1.is_ok(),
            "cell 1 should execute: {:?}",
            result1.err()
        );
        assert_eq!(
            result1.unwrap().as_number(),
            Some(42.0),
            "abs should work via prelude injection"
        );

        // Cell 2: Reference variable from cell 1
        let result2 = execute_repl_command(&mut engine, "x + 1");
        assert!(
            result2.is_ok(),
            "cell 2 should execute: {:?}",
            result2.err()
        );
        assert_eq!(
            result2.unwrap().as_number(),
            Some(43.0),
            "cross-cell reference should work"
        );
    }
}

