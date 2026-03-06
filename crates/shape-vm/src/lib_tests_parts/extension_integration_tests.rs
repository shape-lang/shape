#[cfg(test)]
mod extension_integration_tests {
    use super::*;
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

        // Shape source should be stored as a virtual module at std::loaders::test_ext
        assert!(
            executor
                .virtual_modules
                .contains_key("std::loaders::test_ext"),
            "Extension shape source should be registered as virtual module"
        );
        assert!(
            executor.virtual_modules.contains_key("test_ext"),
            "Extension shape source should also be available at module root path"
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
            executor
                .virtual_modules
                .contains_key("std::loaders::bad_ext"),
            "Even broken source should be registered as virtual module"
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

        // Virtual module should be registered
        assert!(
            executor
                .virtual_modules
                .contains_key("std::loaders::test_ext"),
            "Extension with enum should be registered as virtual module"
        );
        let source = executor
            .virtual_modules
            .get("std::loaders::test_ext")
            .unwrap();
        assert!(
            source.contains("Direction"),
            "Virtual module source should contain enum"
        );
    }

    #[test]
    fn test_extension_module_registered() {
        // Extension module functions should be tracked
        let mut module = shape_runtime::module_exports::ModuleExports::new("test_db");
        module.add_function(
            "load",
            |_args, _ctx: &shape_runtime::module_exports::ModuleContext| {
                Ok(shape_value::ValueWord::none())
            },
        );

        let mut executor = BytecodeExecutor::new();
        let base_count = executor.extensions.len();
        executor.register_extension(module);

        assert_eq!(executor.extensions.len(), base_count + 1);
        let last = executor.extensions.last().unwrap();
        assert_eq!(last.name, "test_db");
        assert!(last.has_export("load"));
    }

    #[test]
    fn test_namespace_call_uses_shape_artifact_exported_function() {
        let mut module = shape_runtime::module_exports::ModuleExports::new("myext");
        module.add_shape_artifact("myext", Some("pub fn connect() { 7 }".to_string()), None);

        let mut executor = BytecodeExecutor::new();
        executor.register_extension(module);

        let mut engine = ShapeEngine::new().expect("engine");
        let result = engine
            .execute(&executor, "use myext\nmyext.connect()")
            .expect("execution should succeed");

        assert_eq!(result.value.as_number(), Some(7.0));
    }

    #[test]
    fn test_shape_artifact_function_can_call_module_namespace_export() {
        let mut module = shape_runtime::module_exports::ModuleExports::new("myext");
        module.add_function(
            "__connect",
            |_args, _ctx: &shape_runtime::module_exports::ModuleContext| {
                Ok(shape_value::ValueWord::from_i64(7))
            },
        );
        module.add_shape_artifact(
            "myext",
            Some("pub fn connect() { myext.__connect() }".to_string()),
            None,
        );

        let mut executor = BytecodeExecutor::new();
        executor.register_extension(module);
        let loader = shape_runtime::module_loader::ModuleLoader::new();
        executor.set_module_loader(loader);
        executor.resolve_file_imports_from_source("use myext\nmyext.connect()", None);

        let mut engine = ShapeEngine::new().expect("engine");
        let result = engine
            .execute(&executor, "use myext\nmyext.connect()")
            .expect("execution should succeed");

        assert_eq!(result.value.as_number(), Some(7.0));
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

        let source = "use myext\nmyext.connect(\"myext://x\")";
        executor.resolve_file_imports_from_source(source, None);

        let program = shape_ast::parser::parse_program(source).expect("parse");
        let mut engine = ShapeEngine::new().expect("engine");
        let bytecode = executor
            .compile_program_for_inspection(&mut engine, &program)
            .expect("compile should succeed");

        let has_specialization = bytecode
            .expanded_function_defs
            .keys()
            .any(|name| name.starts_with("connect__const_"));
        assert!(
            has_specialization,
            "namespace call should trigger const specialization for imported module function"
        );
    }

    #[test]
    fn test_imported_module_comptime_set_return_expr_via_module_export() {
        let mut module = shape_runtime::module_exports::ModuleExports::new("myext");
        module.add_function(
            "__connect_codegen",
            |_args, _ctx: &shape_runtime::module_exports::ModuleContext| {
                Ok(shape_value::ValueWord::from_string(std::sync::Arc::new(
                    "{ __type: string, __uri: string }".to_string(),
                )))
            },
        );
        module.add_shape_artifact(
            "myext",
            Some(
                r#"
annotation db_schema() {
  targets: [function]
  comptime post(target, ctx) {
    set param uri: string
    set return (myext.__connect_codegen(uri))
  }
}
pub @db_schema() fn connect(const uri) { 1 }
"#
                .to_string(),
            ),
            None,
        );

        let mut executor = BytecodeExecutor::new();
        executor.register_extension(module);
        let loader = shape_runtime::module_loader::ModuleLoader::new();
        executor.set_module_loader(loader);

        let source = "use myext\nmyext.connect(\"myext://x\")";
        executor.resolve_file_imports_from_source(source, None);

        let program = shape_ast::parser::parse_program(source).expect("parse");
        let mut engine = ShapeEngine::new().expect("engine");
        let bytecode = executor
            .compile_program_for_inspection(&mut engine, &program)
            .expect("compile should succeed");

        let has_specialization = bytecode
            .expanded_function_defs
            .keys()
            .any(|name| name.starts_with("connect__const_"));
        assert!(
            has_specialization,
            "namespace call should trigger const specialization for set-return-expr handler"
        );
    }

    #[test]
    fn test_imported_module_comptime_handler_can_call_comptime_helper_fn() {
        let mut module = shape_runtime::module_exports::ModuleExports::new("myext");
        module.add_function(
            "__connect_codegen",
            |_args, _ctx: &shape_runtime::module_exports::ModuleContext| {
                Ok(shape_value::ValueWord::from_string(std::sync::Arc::new(
                    "{ __type: string, __uri: string }".to_string(),
                )))
            },
        );
        module.add_shape_artifact(
            "myext",
            Some(
                r#"
comptime fn schema_for(uri) {
  myext.__connect_codegen(uri)
}

annotation db_schema() {
  targets: [function]
  comptime post(target, ctx) {
    set param uri: string
    set return (schema_for(uri))
  }
}
pub @db_schema() fn connect(const uri) { 1 }
"#
                .to_string(),
            ),
            None,
        );

        let mut executor = BytecodeExecutor::new();
        executor.register_extension(module);
        let loader = shape_runtime::module_loader::ModuleLoader::new();
        executor.set_module_loader(loader);

        let source = "use myext\nmyext.connect(\"myext://x\")";
        executor.resolve_file_imports_from_source(source, None);

        let program = shape_ast::parser::parse_program(source).expect("parse");
        let mut engine = ShapeEngine::new().expect("engine");
        let bytecode = executor
            .compile_program_for_inspection(&mut engine, &program)
            .expect("compile should succeed");

        let has_specialization = bytecode
            .expanded_function_defs
            .keys()
            .any(|name| name.starts_with("connect__const_"));
        assert!(
            has_specialization,
            "comptime helper function should be callable from annotation handler"
        );
    }

    #[test]
    fn test_imported_module_typed_callable_field_propagates_table_schema_for_filter_chain() {
        let mut module = shape_runtime::module_exports::ModuleExports::new("myext");
        module.add_function(
            "__connect",
            |_args, _ctx: &shape_runtime::module_exports::ModuleContext| {
                Ok(shape_value::ValueWord::none())
            },
        );
        module.add_function(
            "__connect_codegen",
            |_args, _ctx: &shape_runtime::module_exports::ModuleContext| {
                Ok(shape_value::ValueWord::from_string(std::sync::Arc::new(
                    "{ candles: () => Table<{ open: number }> }".to_string(),
                )))
            },
        );
        module.add_shape_artifact(
            "myext",
            Some(
                r#"
annotation db_schema() {
  targets: [function]
  comptime post(target, ctx) {
    set param uri: string
    set return (myext.__connect_codegen(uri))
  }
}
pub @db_schema() fn connect(const uri: string) { myext.__connect(uri) }
"#
                .to_string(),
            ),
            None,
        );

        let mut executor = BytecodeExecutor::new();
        executor.register_extension(module);
        let loader = shape_runtime::module_loader::ModuleLoader::new();
        executor.set_module_loader(loader);

        let source = r#"
use myext
let conn = myext.connect("myext://x")
let rows = conn.candles().filter(|u| u.open >= 18)
"#;
        executor.resolve_file_imports_from_source(source, None);

        let program = shape_ast::parser::parse_program(source).expect("parse");
        let mut engine = ShapeEngine::new().expect("engine");
        let compiled = executor.compile_program_for_inspection(&mut engine, &program);
        assert!(
            compiled.is_ok(),
            "typed callable field should propagate Table<T> through filter chain: {:?}",
            compiled.err()
        );
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
            executor.virtual_modules.contains_key("std::loaders::ext1"),
            "Should have virtual module for ext1"
        );
        assert!(
            executor.virtual_modules.contains_key("ext1"),
            "Should have root virtual module for ext1"
        );
        assert!(
            executor.virtual_modules.contains_key("std::loaders::ext2"),
            "Should have virtual module for ext2"
        );
        assert!(
            executor.virtual_modules.contains_key("ext2"),
            "Should have root virtual module for ext2"
        );
    }
}

// =========================================================================
// Full Loop Integration Tests: CSV Load → Simulate → Display
// =========================================================================

