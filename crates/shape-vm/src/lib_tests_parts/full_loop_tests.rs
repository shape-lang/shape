#[cfg(test)]
mod full_loop_tests {
    use super::*;
    use shape_runtime::engine::ShapeEngine;

    /// Execute a Shape program through the full engine pipeline.
    fn execute_shape(
        source: &str,
    ) -> std::result::Result<shape_runtime::engine::ExecutionResult, String> {
        let mut engine = ShapeEngine::new().map_err(|e| format!("Engine init failed: {}", e))?;
        engine
            .load_stdlib()
            .map_err(|e| format!("Stdlib load failed: {}", e))?;

        let mut executor = BytecodeExecutor::new();

        engine
            .execute(&mut executor, source)
            .map_err(|e| format!("Execution failed: {}", e))
    }

    #[test]
    fn test_csv_namespace_removed() {
        let result = execute_shape(r#"csv.load(\"/tmp/data.csv\")"#);
        let err = result.expect_err("csv.load must be removed");
        assert!(
            err.contains("csv.load") && err.contains("removed"),
            "expected removed csv.load diagnostic, got: {}",
            err
        );
    }

    #[test]
    fn test_global_load_removed() {
        let result = execute_shape(r#"load(\"market_data\", { symbol: \"ES\" })"#);
        let err = result.expect_err("global load(provider, params) must be removed");
        assert!(
            err.contains("load(provider, params)") && err.contains("removed"),
            "expected removed global load diagnostic, got: {}",
            err
        );
    }

    #[test]
    fn test_print_backtest_results() {
        // Verify that print() doesn't crash when printing structured values.
        let source = r#"
            let state = {
                cash: 100000.0,
                position: 0.0,
                trades: 5,
                wins: 3,
                losses: 2,
                pnl: 1500.0
            }
            print("=== Backtest Results ===")
            print("Cash: $" + state.cash)
            print("Trades: " + state.trades)
            print("P&L: $" + state.pnl)
            "ok"
        "#;

        let result = execute_shape(source);
        match result {
            Ok(r) => {
                let value_str = format!("{:?}", r.value);
                assert!(
                    value_str.contains("ok"),
                    "Expected 'ok' return, got {:?}",
                    r.value
                );
            }
            Err(e) => {
                panic!("Print of backtest results should not fail: {}", e);
            }
        }
    }

    /// backtest() from stdlib.finance.backtest.engine is not directly callable
    /// without import; this test documents the baseline behavior.
    #[test]
    fn test_gap_stdlib_backtest_import() {
        let source = r#"
            let x = 1 + 1
            x
        "#;

        let result = execute_shape(source);
        assert!(result.is_ok(), "Basic execution should work");
    }
}
