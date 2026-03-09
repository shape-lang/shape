//! Main execution methods for Shape engine

use super::types::{ExecutionMetrics, ExecutionResult, ExecutionType};
use shape_ast::error::Result;
use shape_ast::parser;

impl super::ShapeEngine {
    /// Execute a Shape program from source code (sync mode)
    ///
    /// This method allows blocking data loads (REPL mode).
    /// For scripts/backtests, use execute_async() instead.
    pub fn execute(
        &mut self,
        executor: &mut impl super::ProgramExecutor,
        source: &str,
    ) -> Result<ExecutionResult> {
        // Set REPL/sync mode - allow blocking loads
        if let Some(ctx) = self.runtime.persistent_context_mut() {
            ctx.set_data_load_mode(crate::context::DataLoadMode::Sync);
        }

        self.execute_with_options(executor, source, false)
    }

    /// Execute a Shape program with async prefetching (Phase 6/8)
    ///
    /// This method:
    /// 1. Sets Async data mode (runtime data requests use cache, not blocking)
    /// 2. Parses and analyzes the program
    /// 3. Determines required data (symbols/timeframes)
    /// 4. Prefetches data concurrently
    /// 5. Executes synchronously using cached data
    ///
    /// # Example
    ///
    /// ```ignore
    /// let provider = DataFrameAdapter::new(...);
    /// let mut engine = ShapeEngine::with_async_provider(provider)?;
    /// let result = engine.execute_async(&interpreter, "let sma = close.sma(20)").await?;
    /// ```
    pub async fn execute_async(
        &mut self,
        executor: &mut impl super::ProgramExecutor,
        source: &str,
    ) -> Result<ExecutionResult> {
        // Set async mode - runtime data requests must use cache
        if let Some(ctx) = self.runtime.persistent_context_mut() {
            ctx.set_data_load_mode(crate::context::DataLoadMode::Async);
        }

        let start_time = std::time::Instant::now();

        // Parse the source
        let parse_start = std::time::Instant::now();
        let mut program = parser::parse_program(source)?;
        let parse_time_ms = parse_start.elapsed().as_millis() as u64;

        // Desugar high-level syntax (e.g., from-queries to method chains) before analysis
        shape_ast::transform::desugar_program(&mut program);

        let analysis_start = std::time::Instant::now();
        let analysis_time_ms = analysis_start.elapsed().as_millis() as u64;

        // Prefetch data if using async provider
        let has_cache = self
            .runtime
            .persistent_context()
            .map(|ctx| ctx.has_data_cache())
            .unwrap_or(false);

        if has_cache {
            // Extract symbols/timeframes from program
            let queries = self.extract_data_queries(&program)?;

            // Prefetch all required data concurrently
            if let Some(ctx) = self.runtime.persistent_context_mut() {
                ctx.prefetch_data(queries).await?;
            }
        }

        // Store source text for error messages during execution
        self.set_source(source);

        // Execute synchronously using cached data
        let runtime_start = std::time::Instant::now();
        let result = executor.execute_program(self, &program)?;
        let runtime_time_ms = runtime_start.elapsed().as_millis() as u64;

        let total_time_ms = start_time.elapsed().as_millis() as u64;
        let memory_used_bytes = self.estimate_memory_usage();
        let rows_processed = Some(self.default_data.row_count());
        let messages = self.collect_messages();

        Ok(ExecutionResult {
            value: result.wire_value,
            type_info: result.type_info,
            execution_type: result.execution_type,
            metrics: ExecutionMetrics {
                execution_time_ms: total_time_ms,
                parse_time_ms,
                analysis_time_ms,
                runtime_time_ms,
                memory_used_bytes,
                rows_processed,
            },
            messages,
            content_json: result.content_json,
            content_html: result.content_html,
            content_terminal: result.content_terminal,
        })
    }

    /// Execute a REPL command with persistent state
    ///
    /// Unlike `execute_async`, this uses incremental analysis where variables
    /// and functions persist across commands. Call `init_repl()` once before
    /// the first call to this method.
    pub async fn execute_repl(
        &mut self,
        executor: &mut impl super::ProgramExecutor,
        source: &str,
    ) -> Result<ExecutionResult> {
        // Set async mode - runtime data requests must use cache
        if let Some(ctx) = self.runtime.persistent_context_mut() {
            ctx.set_data_load_mode(crate::context::DataLoadMode::Async);
        }

        let start_time = std::time::Instant::now();

        // Parse the source
        let parse_start = std::time::Instant::now();
        let mut program = parser::parse_program(source)?;
        let parse_time_ms = parse_start.elapsed().as_millis() as u64;

        // Desugar high-level syntax (e.g., from-queries to method chains) before analysis
        shape_ast::transform::desugar_program(&mut program);

        let analysis_start = std::time::Instant::now();
        let analysis_time_ms = analysis_start.elapsed().as_millis() as u64;

        // Prefetch data if using async provider
        let has_cache = self
            .runtime
            .persistent_context()
            .map(|ctx| ctx.has_data_cache())
            .unwrap_or(false);

        if has_cache {
            let queries = self.extract_data_queries(&program)?;
            if let Some(ctx) = self.runtime.persistent_context_mut() {
                ctx.prefetch_data(queries).await?;
            }
        }

        // Process imports and declarations before execution
        self.runtime.load_program(&program, &self.default_data)?;

        // Store source text for error messages during execution
        self.set_source(source);

        // Execute
        let runtime_start = std::time::Instant::now();
        let result = executor.execute_program(self, &program)?;
        let runtime_time_ms = runtime_start.elapsed().as_millis() as u64;

        let total_time_ms = start_time.elapsed().as_millis() as u64;
        let memory_used_bytes = self.estimate_memory_usage();
        let rows_processed = Some(self.default_data.row_count());
        let messages = self.collect_messages();

        Ok(ExecutionResult {
            value: result.wire_value,
            type_info: result.type_info,
            execution_type: ExecutionType::Repl,
            metrics: ExecutionMetrics {
                execution_time_ms: total_time_ms,
                parse_time_ms,
                analysis_time_ms,
                runtime_time_ms,
                memory_used_bytes,
                rows_processed,
            },
            messages,
            content_json: result.content_json,
            content_html: result.content_html,
            content_terminal: result.content_terminal,
        })
    }

    /// Parse and analyze source code without executing it.
    ///
    /// Returns the analyzed AST `Program`, ready for compilation.
    /// Used by the recompile-and-resume flow.
    pub fn parse_and_analyze(&mut self, source: &str) -> Result<shape_ast::Program> {
        if let Some(ctx) = self.runtime.persistent_context_mut() {
            ctx.reset_for_new_execution();
        }
        let mut program = parser::parse_program(source)?;
        shape_ast::transform::desugar_program(&mut program);
        self.set_source(source);
        Ok(program)
    }

    /// Execute a Shape program with options
    pub(super) fn execute_with_options(
        &mut self,
        executor: &mut impl super::ProgramExecutor,
        source: &str,
        _is_stdlib: bool,
    ) -> Result<ExecutionResult> {
        let start_time = std::time::Instant::now();

        // Always reset variable scopes before each execution
        if let Some(ctx) = self.runtime.persistent_context_mut() {
            ctx.reset_for_new_execution();
        }

        // Check for deprecated APIs before parsing (the deprecated syntax may not parse cleanly)
        Self::check_deprecated_apis(source)?;

        // Parse the source
        let parse_start = std::time::Instant::now();
        let mut program = parser::parse_program(source)?;
        let parse_time_ms = parse_start.elapsed().as_millis() as u64;

        // Desugar high-level syntax (e.g., from-queries to method chains) before analysis
        shape_ast::transform::desugar_program(&mut program);

        let analysis_start = std::time::Instant::now();
        let analysis_time_ms = analysis_start.elapsed().as_millis() as u64;

        // Store source text for error messages during execution
        self.set_source(source);

        // Execute the program
        let runtime_start = std::time::Instant::now();
        let result = executor.execute_program(self, &program)?;
        let runtime_time_ms = runtime_start.elapsed().as_millis() as u64;

        let total_time_ms = start_time.elapsed().as_millis() as u64;

        // Get memory usage estimate (heap allocation approximation)
        let memory_used_bytes = self.estimate_memory_usage();

        // Get rows processed count from market data
        let rows_processed = Some(self.default_data.row_count());

        // Collect any messages from the runtime
        let messages = self.collect_messages();

        Ok(ExecutionResult {
            value: result.wire_value,
            type_info: result.type_info,
            execution_type: result.execution_type,
            metrics: ExecutionMetrics {
                execution_time_ms: total_time_ms,
                parse_time_ms,
                analysis_time_ms,
                runtime_time_ms,
                memory_used_bytes,
                rows_processed,
            },
            messages,
            content_json: result.content_json,
            content_html: result.content_html,
            content_terminal: result.content_terminal,
        })
    }

    /// Execute a REPL command
    pub fn execute_repl_command(
        &mut self,
        executor: &mut impl super::ProgramExecutor,
        command: &str,
    ) -> Result<ExecutionResult> {
        let mut result = self.execute(executor, command)?;
        result.execution_type = ExecutionType::Repl;
        Ok(result)
    }

    /// Check for deprecated APIs before parsing. Some deprecated call syntax may
    /// not parse cleanly (e.g. escaped quotes in raw strings), so we detect them
    /// via pattern matching on the source text and produce helpful diagnostics.
    fn check_deprecated_apis(source: &str) -> Result<()> {
        let trimmed = source.trim();
        if trimmed.starts_with("csv.load") || trimmed.contains("csv.load(") {
            return Err(shape_ast::error::ShapeError::SemanticError {
                message: "csv.load has been removed. Use the csv package instead: import { read } from \"csv\""
                    .to_string(),
                location: None,
            });
        }
        // Check for bare load(provider, params) — the global load function was removed
        if trimmed.starts_with("load(") || trimmed.starts_with("load (") {
            return Err(shape_ast::error::ShapeError::SemanticError {
                message: "load(provider, params) has been removed. Use typed data access instead: data(\"source\", { ... })"
                    .to_string(),
                location: None,
            });
        }
        Ok(())
    }

    /// Estimate memory usage based on runtime state
    pub(super) fn estimate_memory_usage(&self) -> Option<usize> {
        // Estimate based on known allocations
        let mut total = 0usize;

        // Market data rows (each row ~48 bytes for 6 f64 values)
        total += self.default_data.row_count() * 48;

        // Variable storage estimate (rough approximation)
        // This is a simplified estimate - real tracking would require custom allocator
        total += 1024; // Base overhead for runtime structures

        Some(total)
    }

    /// Collect messages from runtime execution
    pub(super) fn collect_messages(&self) -> Vec<super::types::Message> {
        // Currently the runtime doesn't track messages, but this provides the interface
        // for future implementation
        vec![]
    }
}
