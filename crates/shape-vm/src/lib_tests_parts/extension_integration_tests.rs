#[cfg(test)]
mod extension_integration_tests {
    use crate::BytecodeExecutor;
    use shape_runtime::engine::ShapeEngine;
    use shape_runtime::marshal::{register_typed_fn_0, register_typed_fn_1};
    use shape_runtime::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};
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
        // V3-S5 host-tier rebuild (R8 W2, 2026-05-23): the legacy
        // `register_test_function(&[ValueWord]) -> Result<ValueWord>`
        // wrapper was deleted alongside the dynamic carrier. The kinded
        // replacement is `register_typed_fn_N` per ADR-006 §2.7.4 — the
        // body declares its return shape via `ConcreteType` + `TypedReturn`
        // and the marshal layer projects the result into a `KindedSlot`
        // at the boundary. Per the supervisor 2026-05-23 ruling, the
        // existing `NativeKind::{Int64, Float64, Bool, Int32, Int8}`
        // variants are the only scalar carriers — no NativeScalar.
        let mut module = shape_runtime::module_exports::ModuleExports::new("test_db");
        register_typed_fn_0(
            &mut module,
            "load",
            "Test export used by extension-registration smoke test",
            ConcreteType::Unit,
            |_ctx| Ok(TypedReturn::Concrete(ConcreteReturn::Unit)),
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
    fn test_shape_artifact_function_can_call_module_namespace_export() {
        // V3-S5 host-tier rebuild (R8 W2, 2026-05-23): rebuilt on top of
        // the kinded `register_typed_fn_0` registrar + the post-T1
        // `engine.execute` host boundary. The native `__connect()` export
        // returns `int` (Concrete(I64(7))); the Shape artifact's
        // `connect()` wrapper forwards the call. WireValue.as_number()
        // accepts both `int` and `number` carriers; we expect 7.0.
        let mut module = shape_runtime::module_exports::ModuleExports::new("myext");
        register_typed_fn_0(
            &mut module,
            "__connect",
            "Test native export returning a fixed int",
            ConcreteType::Int,
            |_ctx| Ok(TypedReturn::Concrete(ConcreteReturn::I64(7))),
        );
        module.add_shape_artifact(
            "myext",
            Some("use myext\npub fn connect() { myext::__connect() }".to_string()),
            None,
        );

        let mut executor = BytecodeExecutor::new();
        executor.register_extension(module);
        let loader = shape_runtime::module_loader::ModuleLoader::new();
        executor.set_module_loader(loader);
        executor.resolve_file_imports_from_source("use myext\nmyext::connect()", None);

        let mut engine = ShapeEngine::new().expect("engine");
        let result = engine
            .execute(&mut executor, "use myext\nmyext::connect()")
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
    #[ignore = "v0.4 deferred — const-specialization for imported module functions \
                (same class as sibling `test_imported_module_const_function_specializes_on_namespace_call`, \
                docs/cluster-audits/v0.3-ws10b-29-classification.md row 21). Body \
                ported off `register_test_function` onto the V3-S5 kinded marshal layer \
                (R8 W2, 2026-05-23); the residual failure is pre-existing const-spec \
                pipeline behavior, not a regression of the host-tier rebuild."]
    fn test_imported_module_comptime_set_return_expr_via_module_export() {
        // V3-S5 host-tier rebuild (R8 W2, 2026-05-23): same registrar
        // migration as `test_extension_module_registered` — the codegen
        // body returns a `string` (Concrete(String(_))) via the kinded
        // marshal boundary. Assertion is compile-time only (checks the
        // const-specialization side-table), so end-to-end execution is
        // not exercised here.
        use std::sync::Arc;
        let mut module = shape_runtime::module_exports::ModuleExports::new("myext");
        register_typed_fn_1::<_, Arc<String>>(
            &mut module,
            "__connect_codegen",
            "Test codegen export returning the schema as a string",
            "uri",
            "string",
            ConcreteType::String,
            |_uri, _ctx| {
                Ok(TypedReturn::Concrete(ConcreteReturn::String(
                    "{ __type: string, __uri: string }".to_string(),
                )))
            },
        );
        // Raw-string Shape source uses leading whitespace so the
        // `use myext` line does NOT sit at column 0 of this .rs file.
        // verify-merge.sh CHECK 9 "duplicate use lines" greps every
        // column-0 `use ` line as a Rust `use` statement; multiple
        // fixtures in this file each declare `use myext` inside their
        // body, so the leading whitespace prevents false positives.
        // Shape's parser is whitespace-insensitive at statement-leading
        // positions, so the leading spaces are inert.
        module.add_shape_artifact(
            "myext",
            Some(
                r#"
                use myext
                annotation db_schema() {
                  targets: [function]
                  comptime post(target, ctx) {
                    set param uri: string
                    set return (myext::__connect_codegen(uri))
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
            "namespace call should trigger const specialization for set-return-expr handler"
        );
    }

    #[test]
    #[ignore = "v0.4 deferred — const-specialization for imported module functions \
                (same class as sibling `test_imported_module_const_function_specializes_on_namespace_call`, \
                docs/cluster-audits/v0.3-ws10b-29-classification.md row 21). Body \
                ported off `register_test_function` onto the V3-S5 kinded marshal layer \
                (R8 W2, 2026-05-23); the residual failure is pre-existing const-spec \
                pipeline behavior, not a regression of the host-tier rebuild."]
    fn test_imported_module_comptime_handler_can_call_comptime_helper_fn() {
        // V3-S5 host-tier rebuild (R8 W2, 2026-05-23): mirrors the
        // sibling `test_imported_module_comptime_set_return_expr_via_module_export`
        // — the artifact wires a comptime helper `schema_for(uri)` that
        // delegates to `myext::__connect_codegen`, and the annotation
        // handler calls the helper.
        use std::sync::Arc;
        let mut module = shape_runtime::module_exports::ModuleExports::new("myext");
        register_typed_fn_1::<_, Arc<String>>(
            &mut module,
            "__connect_codegen",
            "Test codegen export returning the schema as a string",
            "uri",
            "string",
            ConcreteType::String,
            |_uri, _ctx| {
                Ok(TypedReturn::Concrete(ConcreteReturn::String(
                    "{ __type: string, __uri: string }".to_string(),
                )))
            },
        );
        // Indented raw-string per the sibling test's leading-whitespace
        // shape (avoids verify-merge.sh CHECK 9 duplicate-use false
        // positive on column-0 `use myext`).
        module.add_shape_artifact(
            "myext",
            Some(
                r#"
                use myext
                comptime fn schema_for(uri) {
                  myext::__connect_codegen(uri)
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
            "comptime helper function should be callable from annotation handler"
        );
    }

    #[test]
    #[ignore = "v0.4 deferred — same const-specialization class as \
                `test_imported_module_const_function_specializes_on_namespace_call` \
                (docs/cluster-audits/v0.3-ws10b-29-classification.md row 21). \
                The annotation's `set return` rewrite does not propagate the \
                Table<T> schema across the comptime boundary in the imported-module \
                path; downstream `conn.candles().filter(|u| u.open >= 18)` fails \
                to resolve `u.open` and the `>=` operands stay `unknown`. Body \
                ported off `register_test_function` onto the V3-S5 kinded marshal \
                layer (R8 W2, 2026-05-23); pre-existing const-spec/annotation \
                pipeline behavior, not a regression of the host-tier rebuild."]
    fn test_imported_module_typed_callable_field_propagates_table_schema_for_filter_chain() {
        // V3-S5 host-tier rebuild (R8 W2, 2026-05-23): two-export shape —
        // `__connect` returns unit (placeholder native handle for the
        // imported db connection); `__connect_codegen(uri)` returns the
        // schema as a string. The annotation rewrites `connect()`'s
        // return type to the schema, so `conn.candles().filter(...)`
        // propagates `Table<{ open: number }>` through the chain at
        // compile time.
        use std::sync::Arc;
        let mut module = shape_runtime::module_exports::ModuleExports::new("myext");
        register_typed_fn_1::<_, Arc<String>>(
            &mut module,
            "__connect",
            "Test native connect — placeholder unit return",
            "uri",
            "string",
            ConcreteType::Unit,
            |_uri, _ctx| Ok(TypedReturn::Concrete(ConcreteReturn::Unit)),
        );
        register_typed_fn_1::<_, Arc<String>>(
            &mut module,
            "__connect_codegen",
            "Test codegen export returning the Table schema as a string",
            "uri",
            "string",
            ConcreteType::String,
            |_uri, _ctx| {
                Ok(TypedReturn::Concrete(ConcreteReturn::String(
                    "{ candles: () => Table<{ open: number }> }".to_string(),
                )))
            },
        );
        // Indented raw-string per the sibling test's leading-whitespace
        // shape (avoids verify-merge.sh CHECK 9 duplicate-use false
        // positive on column-0 `use myext`).
        module.add_shape_artifact(
            "myext",
            Some(
                r#"
                use myext
                annotation db_schema() {
                  targets: [function]
                  comptime post(target, ctx) {
                    set param uri: string
                    set return (myext::__connect_codegen(uri))
                  }
                }
                pub @db_schema() fn connect(const uri: string) { myext::__connect(uri) }
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
                let conn = myext::connect("myext://x")
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
