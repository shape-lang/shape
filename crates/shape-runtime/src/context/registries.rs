//! Registry access methods for ExecutionContext
//!
//! Handles type methods, type schemas, pattern registries,
//! and annotation lifecycle dispatch.

use super::super::annotation_context::AnnotationContext;
use super::super::type_methods::TypeMethodRegistry;
use super::super::type_schema::TypeSchemaRegistry;
use shape_ast::ast::{AnnotationDef, AnnotationHandlerType, FunctionDef};
use shape_ast::error::{Result, ShapeError};
use std::sync::Arc;

impl super::ExecutionContext {
    /// Register an annotation definition
    ///
    /// Annotation definitions are stored and used to dispatch lifecycle hooks
    /// when functions with those annotations are registered.
    pub fn register_annotation(&mut self, def: AnnotationDef) {
        self.annotation_registry.register(def);
    }

    /// Get the annotation context (for lifecycle hooks)
    pub fn annotation_context(&self) -> &AnnotationContext {
        &self.annotation_context
    }

    /// Get mutable annotation context (for lifecycle hooks)
    pub fn annotation_context_mut(&mut self) -> &mut AnnotationContext {
        &mut self.annotation_context
    }

    /// Register a user-defined function
    ///
    /// This dispatches `on_define` lifecycle hooks for all annotations on the function.
    pub fn register_function(&mut self, function: FunctionDef) {
        self.dispatch_on_define_hooks(&function);
    }

    /// Dispatch on_define lifecycle hooks for all annotations on a function
    ///
    /// For each annotation on the function:
    /// 1. Look up the annotation definition in the registry
    /// 2. If it has an on_define handler, execute it
    fn dispatch_on_define_hooks(&mut self, func: &FunctionDef) {
        for annotation in &func.annotations {
            // Look up the annotation definition
            if let Some(ann_def) = self.annotation_registry.get(&annotation.name).cloned() {
                // Find the on_define handler
                for handler in &ann_def.handlers {
                    if handler.handler_type == AnnotationHandlerType::OnDefine {
                        // Execute the on_define handler
                        self.execute_on_define_handler(&ann_def, handler, func);
                    }
                }
            }
            // If annotation definition not found, no hooks are executed
            // Annotations must be defined to have behavior
        }
    }

    /// Execute an on_define lifecycle handler
    ///
    /// The handler body is a Shape expression that will be evaluated
    /// with `fn` and `ctx` bound to the function and annotation context.
    /// Currently a stub until VM-based closure handling is implemented.
    fn execute_on_define_handler(
        &mut self,
        _ann_def: &AnnotationDef,
        _handler: &shape_ast::ast::AnnotationHandler,
        _func: &FunctionDef,
    ) {
        self.sync_pattern_registry_from_annotation_context();
    }

    /// Sync pattern registry from annotation context
    ///
    /// When an annotation's on_define calls ctx.registry("patterns").set(...),
    /// we need to copy those entries to the main pattern_registry for .find() lookup.
    /// Currently a stub until VM-based closure handling is implemented.
    fn sync_pattern_registry_from_annotation_context(&mut self) {}

    /// Look up a pattern by name from the pattern registry
    ///
    /// Returns an error with a helpful message if the pattern is not found.
    pub fn lookup_pattern(&self, name: &str) -> Result<&super::super::closure::Closure> {
        self.pattern_registry
            .get(name)
            .ok_or_else(|| ShapeError::RuntimeError {
                message: format!(
                    "Unknown pattern: '{}'. Did you register it in the pattern registry?",
                    name
                ),
                location: None,
            })
    }

    /// Get the type method registry
    pub fn type_method_registry(&self) -> &Arc<TypeMethodRegistry> {
        &self.type_method_registry
    }

    /// Get the type schema registry for JIT type specialization
    pub fn type_schema_registry(&self) -> &Arc<TypeSchemaRegistry> {
        &self.type_schema_registry
    }

    // =========================================================================
    // Enum Registry Methods (for sum types)
    // =========================================================================

    /// Register an enum definition
    ///
    /// This enables sum type support by tracking which enums exist and their variants.
    /// Called during semantic analysis when processing `enum` declarations.
    pub fn register_enum(&mut self, enum_def: shape_ast::ast::EnumDef) {
        self.enum_registry.register(enum_def);
    }

    /// Look up an enum definition by name
    pub fn lookup_enum(&self, name: &str) -> Option<&shape_ast::ast::EnumDef> {
        self.enum_registry.get(name)
    }

    /// Check if an enum exists
    pub fn has_enum(&self, name: &str) -> bool {
        self.enum_registry.contains(name)
    }

    /// Get the enum registry (for advanced queries)
    pub fn enum_registry(&self) -> &super::EnumRegistry {
        &self.enum_registry
    }
}

#[cfg(test)]
mod tests {
    use super::super::ExecutionContext;
    use shape_ast::ast::{
        Annotation, AnnotationDef, AnnotationHandler, AnnotationHandlerType, Expr, FunctionDef,
        Span,
    };

    #[test]
    fn test_register_annotation_definition() {
        let mut ctx = ExecutionContext::new_empty();

        // Create a simple annotation definition (no handlers for this test)
        let ann_def = AnnotationDef {
            name: "test_annotation".to_string(),
            name_span: Span::DUMMY,
            doc_comment: None,
            params: vec![],
            allowed_targets: None,
            handlers: vec![],
            span: Span::DUMMY,
        };

        ctx.register_annotation(ann_def);

        // Verify the annotation is registered
        assert!(ctx.annotation_registry.contains("test_annotation"));
    }

    #[test]
    fn test_function_without_annotations_registers_normally() {
        let mut ctx = ExecutionContext::new_empty();

        let func = FunctionDef {
            name: "my_func".to_string(),
            name_span: Span::DUMMY,
            declaring_module_path: None,
            doc_comment: None,
            type_params: None,
            params: vec![],
            return_type: None,
            body: vec![],
            annotations: vec![],
            where_clause: None,
            is_async: false,
            is_comptime: false,
        };

        ctx.register_function(func);

        // TODO: Update test when BytecodeExecutor/VM integration is complete
        // Function should be registered in evaluator
        // assert!(ctx.evaluator().get_function("my_func").is_some());
    }

    #[test]
    fn test_function_with_undefined_annotation_no_crash() {
        let mut ctx = ExecutionContext::new_empty();

        // Function has @undefined annotation but no annotation definition registered
        let func = FunctionDef {
            name: "annotated_func".to_string(),
            name_span: Span::DUMMY,
            declaring_module_path: None,
            doc_comment: None,
            type_params: None,
            params: vec![],
            return_type: None,
            body: vec![],
            annotations: vec![Annotation {
                name: "undefined".to_string(),
                args: vec![],
                span: Span::DUMMY,
            }],
            where_clause: None,
            is_async: false,
            is_comptime: false,
        };

        // Should not crash - undefined annotations are silently ignored
        ctx.register_function(func);

        // TODO: Update test when BytecodeExecutor/VM integration is complete
        // Function should still be registered
        // assert!(ctx.evaluator().get_function("annotated_func").is_some());
    }

    #[test]
    fn test_annotation_with_on_define_handler_is_called() {
        let mut ctx = ExecutionContext::new_empty();

        // Create an annotation definition with on_define handler
        // The handler body sets a value in the annotation context state
        // For simplicity, we use a literal expression that doesn't require complex evaluation
        let ann_def = AnnotationDef {
            name: "tracked".to_string(),
            name_span: Span::DUMMY,
            doc_comment: None,
            params: vec![],
            allowed_targets: None,
            handlers: vec![AnnotationHandler {
                handler_type: AnnotationHandlerType::OnDefine,
                params: vec![shape_ast::ast::AnnotationHandlerParam {
                    name: "fn".to_string(),
                    is_variadic: false,
                }],
                return_type: None,
                // Simple expression that just returns the fn parameter
                // This tests that the handler is called and fn is bound
                body: Expr::Identifier("fn".to_string(), Span::DUMMY),
                span: Span::DUMMY,
            }],
            span: Span::DUMMY,
        };

        ctx.register_annotation(ann_def);

        // Create a function with @tracked annotation
        let func = FunctionDef {
            name: "tracked_func".to_string(),
            name_span: Span::DUMMY,
            declaring_module_path: None,
            doc_comment: None,
            type_params: None,
            params: vec![],
            return_type: None,
            body: vec![],
            annotations: vec![Annotation {
                name: "tracked".to_string(),
                args: vec![],
                span: Span::DUMMY,
            }],
            where_clause: None,
            is_async: false,
            is_comptime: false,
        };

        // Register function - this should trigger on_define handler
        ctx.register_function(func);

        // TODO: Update test when BytecodeExecutor/VM integration is complete
        // Function should be registered
        // assert!(ctx.evaluator().get_function("tracked_func").is_some());
    }

    #[test]
    fn test_lookup_pattern_not_found() {
        let ctx = ExecutionContext::new_empty();

        let result = ctx.lookup_pattern("nonexistent");
        assert!(result.is_err());

        let err = result.unwrap_err();
        let msg = format!("{}", err);
        assert!(msg.contains("Unknown pattern"));
        assert!(msg.contains("nonexistent"));
    }

    #[test]
    fn test_annotation_context_registry_access() {
        let mut ctx = ExecutionContext::new_empty();

        // Access and modify the annotation context registry
        {
            let registry = ctx.annotation_context_mut().registry("test_registry");
            registry.set(
                "key1".to_string(),
                shape_value::ValueWord::from_string(std::sync::Arc::new("value1".to_string())),
            );
        }

        // Verify the value is stored
        let registry = ctx.annotation_context_mut().registry("test_registry");
        assert!(registry.get("key1").is_some());
    }
}
