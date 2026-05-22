//! Main execution methods for Shape engine

use super::types::{ExecutionMetrics, ExecutionResult, ExecutionType};
use crate::type_schema::with_async_scope;
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

        // Install this runtime's TypeSchemaRegistry as the task-local
        // ambient for the duration of this execution.
        let schema_registry = self.runtime.schema_registry_arc();
        with_async_scope(schema_registry, self.execute_async_inner(executor, source)).await
    }

    async fn execute_async_inner(
        &mut self,
        executor: &mut impl super::ProgramExecutor,
        source: &str,
    ) -> Result<ExecutionResult> {
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

        // Install this runtime's TypeSchemaRegistry as the task-local
        // ambient for the duration of this REPL command.
        let schema_registry = self.runtime.schema_registry_arc();
        with_async_scope(schema_registry, self.execute_repl_inner(executor, source)).await
    }

    async fn execute_repl_inner(
        &mut self,
        executor: &mut impl super::ProgramExecutor,
        source: &str,
    ) -> Result<ExecutionResult> {
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

        // Process imports and declarations before execution.
        //
        // REPL cross-cell persistence (WS-11): definition-item injection
        // and module-binding round-trip both happen inside
        // `ProgramExecutor::execute_program` (it owns the only
        // cross-cell-stable handle — `ShapeEngine` — and is the single
        // path shared by every executor caller). `load_program` only
        // needs the cell's own items; the executor re-injects prior
        // definitions before compilation.
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
        // Parse/desugar may touch ambient schema state (e.g. comptime
        // builtins). Install this runtime's registry for the call.
        let _scope = self.runtime.enter_schema_scope();

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
        // Install this runtime's TypeSchemaRegistry as the thread-local
        // ambient for the duration of synchronous execution.
        let _scope = self.runtime.enter_schema_scope();

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

    /// Definition items accumulated from prior REPL cells (WS-11).
    ///
    /// The `ProgramExecutor` re-prepends these to each new cell's
    /// program before bytecode compilation so that `fn` / `type` /
    /// `enum` / `trait` / `impl` / type-alias / annotation declarations
    /// from earlier lines remain resolvable. Empty unless [`init_repl`]
    /// enabled cross-cell persistence.
    pub fn repl_definitions(&self) -> &[shape_ast::ast::Item] {
        &self.repl_definitions
    }

    /// Whether cross-cell persistence is active AND there is at least
    /// one accumulated definition to inject.
    pub fn has_repl_definitions(&self) -> bool {
        self.repl_persistence && !self.repl_definitions.is_empty()
    }

    /// Harvest a successfully-executed cell's definition items into the
    /// cross-cell accumulator (WS-11).
    ///
    /// Call this only after the cell ran without error, so a failed cell
    /// does not poison the accumulated state. No-op when cross-cell
    /// persistence is disabled.
    pub fn absorb_repl_cell_definitions(&mut self, program: &shape_ast::Program) {
        if !self.repl_persistence {
            return;
        }
        let cell = Self::collect_definition_items(program);
        if !cell.is_empty() {
            self.absorb_repl_definitions(cell);
        }
    }

    /// User type schemas persisted across REPL cells, keyed by name
    /// (WS-11). The `ProgramExecutor` seeds each cell's compiler with
    /// these so a `type` keeps a stable `SchemaId` for the whole
    /// session.
    pub fn repl_user_schemas(
        &self,
    ) -> &std::collections::HashMap<String, crate::type_schema::TypeSchema> {
        &self.repl_user_schemas
    }

    /// Record a user type schema under its first-assigned id (WS-11).
    ///
    /// Idempotent and first-write-wins: once a `type` has a session id,
    /// later cells must not overwrite it — that id is what every already
    /// persisted instance of the type carries.
    pub fn remember_repl_user_schema(&mut self, schema: crate::type_schema::TypeSchema) {
        if !self.repl_persistence {
            return;
        }
        self.repl_user_schemas
            .entry(schema.name.clone())
            .or_insert(schema);
    }

    /// Names of the struct / enum types this cell's definition items
    /// introduce — used to pull the matching schemas out of the
    /// compiled program's registry for cross-cell id stabilization.
    pub fn repl_user_type_names(program: &shape_ast::Program) -> Vec<String> {
        use shape_ast::ast::Item;
        let mut names = Vec::new();
        for item in &program.items {
            match item {
                Item::StructType(s, _) => names.push(s.name.clone()),
                Item::Enum(e, _) => names.push(e.name.clone()),
                _ => {}
            }
        }
        names
    }

    /// Collect the definition-class items from a parsed REPL cell.
    ///
    /// "Definition" here means an item whose effect is to introduce a
    /// reusable name (a function, type, enum, trait, impl, type alias,
    /// annotation, or declaration-only builtin) rather than to compute a
    /// value. These are the items that must survive into subsequent
    /// cells so that later lines can reference them. Statement-class
    /// items (`let`, expressions, assignments) are intentionally
    /// excluded — their *values* persist via the module-binding
    /// round-trip in the program executor, not via AST re-injection.
    fn collect_definition_items(program: &shape_ast::Program) -> Vec<shape_ast::ast::Item> {
        use shape_ast::ast::Item;
        program
            .items
            .iter()
            .filter(|item| {
                matches!(
                    item,
                    Item::Function(..)
                        | Item::StructType(..)
                        | Item::Enum(..)
                        | Item::Trait(..)
                        | Item::Impl(..)
                        | Item::Extend(..)
                        | Item::TypeAlias(..)
                        | Item::AnnotationDef(..)
                        | Item::ForeignFunction(..)
                        | Item::BuiltinTypeDecl(..)
                        | Item::BuiltinFunctionDecl(..)
                )
            })
            .cloned()
            .collect()
    }

    /// Fold a cell's definition items into the cross-cell accumulator.
    ///
    /// A later redefinition of a name (`fn foo` after an earlier
    /// `fn foo`, `type Point` after an earlier `type Point`) replaces
    /// the earlier definition rather than accumulating a duplicate that
    /// would make the next cell's compilation ambiguous. Items keyed by
    /// the same identity are matched by name; `impl` / `extend` blocks
    /// are keyed by their target (and trait/impl selector) so a
    /// re-`impl` of the same trait-for-type pair supersedes the prior
    /// one.
    fn absorb_repl_definitions(&mut self, cell: Vec<shape_ast::ast::Item>) {
        for item in cell {
            let key = Self::definition_identity(&item);
            if let Some(pos) = self
                .repl_definitions
                .iter()
                .position(|existing| Self::definition_identity(existing) == key)
            {
                self.repl_definitions[pos] = item;
            } else {
                self.repl_definitions.push(item);
            }
        }
    }

    /// A stable identity string for a definition item, used to dedup the
    /// cross-cell accumulator. Two items with the same identity are the
    /// "same declaration" for redefinition purposes.
    fn definition_identity(item: &shape_ast::ast::Item) -> String {
        use shape_ast::ast::Item;
        match item {
            Item::Function(f, _) => format!("fn:{}", f.name),
            Item::StructType(s, _) => format!("type:{}", s.name),
            Item::Enum(e, _) => format!("enum:{}", e.name),
            Item::Trait(t, _) => format!("trait:{}", t.name),
            Item::TypeAlias(a, _) => format!("alias:{}", a.name),
            Item::AnnotationDef(a, _) => format!("ann:{}", a.name),
            Item::ForeignFunction(f, _) => format!("fn:{}", f.name),
            Item::BuiltinTypeDecl(d, _) => format!("type:{}", d.name),
            Item::BuiltinFunctionDecl(d, _) => format!("fn:{}", d.name),
            Item::Impl(i, _) => format!(
                "impl:{}:{}:{}",
                Self::type_name_key(&i.trait_name),
                Self::type_name_key(&i.target_type),
                i.impl_name.as_deref().unwrap_or(""),
            ),
            Item::Extend(e, _) => {
                format!("extend:{}", Self::type_name_key(&e.type_name))
            }
            // Non-definition items are never absorbed; give each a unique
            // identity so they would never collide if one slipped through.
            other => format!("other:{:p}", other as *const _),
        }
    }

    /// Base path name of a `TypeName`, used as a dedup key component for
    /// `impl` / `extend` blocks in the cross-cell definition accumulator.
    fn type_name_key(tn: &shape_ast::ast::TypeName) -> &str {
        match tn {
            shape_ast::ast::TypeName::Simple(path) => path.name(),
            shape_ast::ast::TypeName::Generic { name, .. } => name.name(),
        }
    }
}
