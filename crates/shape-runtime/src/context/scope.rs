//! Scope management for ExecutionContext
//!
//! Handles pushing/popping scopes for function calls and creating parallel execution contexts.

use std::collections::HashMap;
use std::sync::Arc;

impl super::ExecutionContext {
    /// Push a new variable scope (for function calls)
    pub fn push_scope(&mut self) {
        self.variable_scopes.push(HashMap::new());
    }

    /// Pop a variable scope (after function return)
    pub fn pop_scope(&mut self) {
        if self.variable_scopes.len() > 1 {
            self.variable_scopes.pop();
        }
    }

    /// Reset for new execution (Interpreter mode)
    ///
    /// Clears script-level variables while preserving:
    /// - Data cache (loaded instruments)
    /// - Registered functions (from stdlib)
    /// - Data providers
    /// - Module exports
    ///
    /// This allows re-execution of code with the same variable names
    /// without "Variable already declared" errors.
    pub fn reset_for_new_execution(&mut self) {
        // Clear the root scope but keep the structure
        if let Some(root_scope) = self.variable_scopes.first_mut() {
            root_scope.clear();
        }
        // Ensure we only have the root scope (remove any leftover nested scopes)
        self.variable_scopes.truncate(1);
    }

    /// Create a fresh context for parallel execution that shares Arc-wrapped resources
    /// but has independent mutable state (variables, position manager, caches)
    pub fn fork_for_parallel(&self) -> Self {
        Self {
            // Shared immutable resources (Arc-wrapped)
            data_provider: self.data_provider.clone(),
            data_cache: None,
            provider_registry: Arc::new(super::super::provider_registry::ProviderRegistry::new()),
            type_mapping_registry: Arc::new(super::super::type_mapping::TypeMappingRegistry::new()),
            type_schema_registry: self.type_schema_registry.clone(),
            metadata_registry: self.metadata_registry.clone(),
            data_load_mode: super::DataLoadMode::default(),
            type_method_registry: self.type_method_registry.clone(),

            // Copied configuration
            current_id: self.current_id.clone(),
            current_row_index: self.current_row_index,
            variable_scopes: self.variable_scopes.clone(),
            reference_datetime: self.reference_datetime,
            current_timeframe: self.current_timeframe,
            base_timeframe: self.base_timeframe,
            lookahead_guard: self.lookahead_guard.clone(),
            date_range: self.date_range,
            range_start: self.range_start,
            range_end: self.range_end,
            range_active: self.range_active,
            pattern_registry: self.pattern_registry.clone(),
            annotation_context: self.annotation_context.clone(),
            annotation_registry: self.annotation_registry.clone(),
            event_queue: self.event_queue.clone(),
            suspension_state: self.suspension_state.clone(),
            alert_pipeline: self.alert_pipeline.clone(),
            output_adapter: self.output_adapter.clone(),
            type_alias_registry: self.type_alias_registry.clone(),
            enum_registry: self.enum_registry.clone(),
            struct_type_registry: self.struct_type_registry.clone(),
            progress_registry: self.progress_registry.clone(),
            kernel_compiler: self.kernel_compiler.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::ExecutionContext;

    #[test]
    fn test_execution_context_scope_push_pop() {
        let mut ctx = ExecutionContext::new_empty();

        // Push and pop scopes
        ctx.push_scope();
        ctx.pop_scope();
        // Should not panic
    }
}
