#[cfg(test)]
mod extension_integration_tests {
    use super::*;
    use crate::BytecodeExecutor;
    use shape_runtime::engine::ShapeEngine;
    #[test]
    fn test_extension_shape_source_registered_as_virtual_module() {
        // Register an extension that bundles a Shape source
        let mut module = shape_runtime::module_exports::ModuleExports::new("test_ext");
        module.add_shape_source(
            "helpers.shape",
            r#"
            pub fn ext_double(x) { x * 2 }
        "#,
        );

        let mut executor = BytecodeExecutor::new();
        executor.register_extension(module);

        // Shape source should be stored as a virtual module under the module's canonical name.
        assert!(
            executor.virtual_modules.contains_key("test_ext"),
            "Extension shape source should be registered under canonical name"
        );
    }

    #[test]
    fn test_extension_shape_source_parse_error_deferred() {
        // Extension with invalid Shape code is stored as virtual module
        // (error surfaces at import time, not registration time)
        let mut module = shape_runtime::module_exports::ModuleExports::new("bad_ext");
        module.add_shape_source("broken.shape", "fn broken(( { }");

        let mut executor = BytecodeExecutor::new();
        executor.register_extension(module);

        // Virtual module is still registered (error happens when imported)
        assert!(
            executor.virtual_modules.contains_key("bad_ext"),
            "Even broken source should be registered under canonical name"
        );
    }

    #[test]
    fn test_extension_with_enum_registered_as_virtual_module() {
        let mut module = shape_runtime::module_exports::ModuleExports::new("test_ext");
        module.add_shape_source(
            "test.shape",
            r#"
            pub enum Direction { Up, Down }
            pub fn ext_direction_name(d) {
                match d {
                    Direction::Up => "up",
                    Direction::Down => "down"
                }
            }
        "#,
        );

        let mut executor = BytecodeExecutor::new();
        executor.register_extension(module);

        // Virtual module should be registered under canonical name
        assert!(
            executor.virtual_modules.contains_key("test_ext"),
            "Extension with enum should be registered under canonical name"
        );
        let source = executor.virtual_modules.get("test_ext").unwrap();
        assert!(
            source.contains("Direction"),
            "Virtual module source should contain enum"
        );
    }

    #[test]
    fn test_extension_module_registered() {
        todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
    }

    #[test]
    fn test_shape_artifact_function_can_call_module_namespace_export() {
        todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
    }

    #[test]
    fn test_imported_module_const_function_specializes_on_namespace_call() {
        let mut module = shape_runtime::module_exports::ModuleExports::new("myext");
        module.add_shape_artifact(
            "myext",
            Some(
                r#"
annotation force_int() {
  comptime post(target, ctx) {
    set return int
  }
}
pub @force_int() fn connect(const uri) { 1 }
"#
                .to_string(),
            ),
            None,
        );

        let mut executor = BytecodeExecutor::new();
        executor.register_extension(module);
        let loader = shape_runtime::module_loader::ModuleLoader::new();
        executor.set_module_loader(loader);

        let source = "use myext\nmyext::connect(\"myext://x\")";
        executor.resolve_file_imports_from_source(source, None);

        let program = shape_ast::parser::parse_program(source).expect("parse");
        let mut engine = ShapeEngine::new().expect("engine");
        let bytecode = executor
            .compile_program_for_inspection(&mut engine, &program)
            .expect("compile should succeed");

        let has_specialization = bytecode
            .expanded_function_defs
            .keys()
            .any(|name| name.contains("connect__const_"));
        assert!(
            has_specialization,
            "namespace call should trigger const specialization for imported module function"
        );
    }

    #[test]
    fn test_imported_module_comptime_set_return_expr_via_module_export() {
        todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
    }

    #[test]
    fn test_imported_module_comptime_handler_can_call_comptime_helper_fn() {
        todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
    }

    #[test]
    fn test_imported_module_typed_callable_field_propagates_table_schema_for_filter_chain() {
        todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
    }

    #[test]
    fn test_multiple_extensions_register_separate_virtual_modules() {
        let mut ext1 = shape_runtime::module_exports::ModuleExports::new("ext1");
        ext1.add_shape_source("a.shape", "pub fn ext1_fn() { 1 }");

        let mut ext2 = shape_runtime::module_exports::ModuleExports::new("ext2");
        ext2.add_shape_source("b.shape", "pub fn ext2_fn() { 2 }");

        let mut executor = BytecodeExecutor::new();
        executor.register_extension(ext1);
        executor.register_extension(ext2);

        assert!(
            executor.virtual_modules.contains_key("ext1"),
            "Should have virtual module for ext1"
        );
        assert!(
            executor.virtual_modules.contains_key("ext2"),
            "Should have virtual module for ext2"
        );
    }
}

// =========================================================================
// Full Loop Integration Tests: CSV Load → Simulate → Display
// =========================================================================
