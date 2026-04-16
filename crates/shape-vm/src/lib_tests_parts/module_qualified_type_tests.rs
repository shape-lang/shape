#[cfg(test)]
mod module_qualified_type_tests {
    use crate::compiler::BytecodeCompiler;
    use crate::executor::{VMConfig, VirtualMachine};
    use shape_value::{ValueWord, ValueWordExt};

    fn eval(code: &str) -> ValueWord {
        let program = shape_ast::parser::parse_program(code).expect("parse failed");
        let compiler = BytecodeCompiler::new();
        let bytecode = compiler.compile(&program).expect("compile failed");
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        vm.populate_module_objects();
        vm.execute(None).expect("execution failed").clone()
    }

    // ===== Parser tests for qualified types =====

    #[test]
    fn test_parse_qualified_type_reference() {
        let source = "let x: foo::Bar = 1";
        let program = shape_ast::parser::parse_program(source).expect("parse");
        let items = &program.items;
        if let shape_ast::ast::Item::Statement(shape_ast::ast::Statement::VariableDecl(decl, _), _) = &items[0] {
            match &decl.type_annotation {
                Some(shape_ast::ast::TypeAnnotation::Reference(path)) => {
                    assert_eq!(path.as_str(), "foo::Bar");
                    assert!(path.is_qualified());
                    assert_eq!(path.name(), "Bar");
                }
                other => panic!("Expected Reference(foo::Bar), got {:?}", other),
            }
        } else {
            panic!("Expected VariableDecl");
        }
    }

    #[test]
    fn test_parse_qualified_generic_type() {
        let source = "let x: foo::Container<int> = 1";
        let program = shape_ast::parser::parse_program(source).expect("parse");
        let items = &program.items;
        if let shape_ast::ast::Item::Statement(shape_ast::ast::Statement::VariableDecl(decl, _), _) = &items[0] {
            match &decl.type_annotation {
                Some(shape_ast::ast::TypeAnnotation::Generic { name, args }) => {
                    assert_eq!(name.as_str(), "foo::Container");
                    assert!(name.is_qualified());
                    assert_eq!(args.len(), 1);
                }
                other => panic!("Expected Generic(foo::Container), got {:?}", other),
            }
        } else {
            panic!("Expected VariableDecl");
        }
    }

    #[test]
    fn test_parse_qualified_enum_constructor() {
        let source = "let c = types::Color::Red";
        let program = shape_ast::parser::parse_program(source).expect("parse");
        let items = &program.items;
        if let shape_ast::ast::Item::Statement(shape_ast::ast::Statement::VariableDecl(decl, _), _) = &items[0] {
            match &decl.value {
                Some(shape_ast::ast::Expr::EnumConstructor { enum_name, variant, .. }) => {
                    assert_eq!(enum_name.as_str(), "types::Color");
                    assert_eq!(variant, "Red");
                }
                other => panic!("Expected EnumConstructor, got {:?}", other.as_ref().map(std::mem::discriminant)),
            }
        } else {
            panic!("Expected VariableDecl");
        }
    }

    #[test]
    fn test_parse_deeply_qualified_enum_constructor() {
        let source = "let c = a::b::Color::Red";
        let program = shape_ast::parser::parse_program(source).expect("parse");
        let items = &program.items;
        if let shape_ast::ast::Item::Statement(shape_ast::ast::Statement::VariableDecl(decl, _), _) = &items[0] {
            match &decl.value {
                Some(shape_ast::ast::Expr::EnumConstructor { enum_name, variant, .. }) => {
                    assert_eq!(enum_name.as_str(), "a::b::Color");
                    assert_eq!(variant, "Red");
                }
                other => panic!("Expected EnumConstructor, got {:?}", other.as_ref().map(std::mem::discriminant)),
            }
        } else {
            panic!("Expected VariableDecl");
        }
    }

    #[test]
    fn test_parse_qualified_pattern_constructor() {
        let source = "match x { types::Color::Red => 1 }";
        let program = shape_ast::parser::parse_program(source).expect("parse");
        // Verify it parses successfully with the qualified pattern
        assert!(!program.items.is_empty());
    }

    // ===== Eval tests for module-qualified types =====

    #[test]
    fn test_module_struct_literal_qualified() {
        // m::P { x: 42 } parses as EnumConstructor(enum="m", variant="P", payload=Struct)
        // The compiler's enum→struct fallback in compile_expr_enum_constructor handles this
        let result = eval(r#"
            mod m { type P { x: int } }
            m::P { x: 42 }.x
        "#);
        assert_eq!(result.as_i64(), Some(42));
    }

    #[test]
    fn test_module_enum_constructor_and_match() {
        let result = eval(r#"
            mod m { enum C { R, B } }
            match m::C::R {
                m::C::R => 1,
                m::C::B => 2,
            }
        "#);
        assert_eq!(result.as_i64(), Some(1));
    }

    #[test]
    fn test_module_extend_method() {
        let result = eval(r#"
            mod m {
                type P { x: int }
                extend P {
                    method dbl() -> int { self.x * 2 }
                }
            }
            m::P { x: 5 }.dbl()
        "#);
        assert_eq!(result.as_i64(), Some(10));
    }

    #[test]
    fn test_module_unqualified_access_inside() {
        let result = eval(r#"
            mod m {
                type P { x: int }
                fn mk() -> P { P { x: 3 } }
            }
            m::mk().x
        "#);
        assert_eq!(result.as_i64(), Some(3));
    }

    #[test]
    fn test_module_enum_tuple_payload() {
        let result = eval(r#"
            mod m { enum S { C(int) } }
            match m::S::C(7) {
                m::S::C(n) => n,
            }
        "#);
        assert_eq!(result.as_i64(), Some(7));
    }

    #[test]
    fn test_module_impl_trait() {
        let result = eval(r#"
            mod m {
                trait Greet { greet(self): string }
                type P { name: string }
                impl Greet for P {
                    method greet() -> string { self.name }
                }
            }
            m::P { name: "hi" }.greet()
        "#);
        assert_eq!(
            result.as_arc_string().expect("Expected String").as_ref() as &str,
            "hi"
        );
    }

    // ===== Additional integration tests =====

    #[test]
    fn test_module_type_alias() {
        // Type aliases inside modules should be qualified to m::Alias
        let result = eval(r#"
            mod m {
                type Alias = int
                fn make() -> Alias { 99 }
            }
            m::make()
        "#);
        assert_eq!(result.as_i64(), Some(99));
    }

    #[test]
    fn test_module_enum_struct_variant() {
        // Enum struct variants should work with qualified names
        let result = eval(r#"
            mod m {
                enum E { V { x: int, y: int } }
            }
            match m::E::V { x: 1, y: 2 } {
                m::E::V { x, y } => x + y,
            }
        "#);
        assert_eq!(result.as_i64(), Some(3));
    }

    #[test]
    fn test_module_multiple_types() {
        // Multiple types in the same module should all be qualified independently
        let result = eval(r#"
            mod m {
                type A { x: int }
                type B { y: int }
                fn sum(a: A, b: B) -> int { a.x + b.y }
            }
            m::sum(m::A { x: 10 }, m::B { y: 20 })
        "#);
        assert_eq!(result.as_i64(), Some(30));
    }

    #[test]
    fn test_module_enum_used_in_function_signature() {
        // Module-qualified enum used as function return type
        let result = eval(r#"
            mod m {
                enum Color { Red, Blue }
                fn pick() -> Color { Color::Red }
            }
            match m::pick() {
                m::Color::Red => 1,
                m::Color::Blue => 2,
            }
        "#);
        assert_eq!(result.as_i64(), Some(1));
    }

    #[test]
    fn test_module_struct_with_method_chaining() {
        // Extend method chaining on module-qualified types
        let result = eval(r#"
            mod m {
                type Counter { n: int }
                extend Counter {
                    method inc() -> Counter { Counter { n: self.n + 1 } }
                    method value() -> int { self.n }
                }
            }
            m::Counter { n: 0 }.inc().inc().inc().value()
        "#);
        assert_eq!(result.as_i64(), Some(3));
    }

    #[test]
    fn test_module_type_in_let_binding_annotation() {
        // Qualified type annotation in let binding
        let result = eval(r#"
            mod m { type P { x: int } }
            let p: m::P = m::P { x: 7 }
            p.x
        "#);
        assert_eq!(result.as_i64(), Some(7));
    }

    // ===== Phase B: qualified trait bounds in dyn/type params =====

    #[test]
    fn test_parse_qualified_dyn_type() {
        let source = "let x: dyn foo::Bar = 1";
        let program = shape_ast::parser::parse_program(source).expect("parse");
        let items = &program.items;
        if let shape_ast::ast::Item::Statement(shape_ast::ast::Statement::VariableDecl(decl, _), _) = &items[0] {
            match &decl.type_annotation {
                Some(shape_ast::ast::TypeAnnotation::Dyn(traits)) => {
                    assert_eq!(traits.len(), 1);
                    assert_eq!(traits[0].as_str(), "foo::Bar");
                }
                other => panic!("Expected Dyn(foo::Bar), got {:?}", other),
            }
        } else {
            panic!("Expected VariableDecl");
        }
    }

    #[test]
    fn test_parse_qualified_trait_bound() {
        let source = r#"
            fn foo<T: mod1::Comparable>(x: T) -> T { x }
        "#;
        let program = shape_ast::parser::parse_program(source).expect("parse");
        if let shape_ast::ast::Item::Function(func, _) = &program.items[0] {
            let tp = &func.type_params.as_ref().expect("type params")[0];
            assert_eq!(tp.name, "T");
            assert_eq!(tp.trait_bounds.len(), 1);
            assert_eq!(tp.trait_bounds[0].as_str(), "mod1::Comparable");
        } else {
            panic!("Expected Function");
        }
    }

    #[test]
    fn test_parse_qualified_where_clause_bound() {
        let source = r#"
            fn foo<T>(x: T) -> T where T: mod1::Printable + mod2::Serializable { x }
        "#;
        let program = shape_ast::parser::parse_program(source).expect("parse");
        if let shape_ast::ast::Item::Function(func, _) = &program.items[0] {
            let wc = func.where_clause.as_ref().expect("where clause");
            assert_eq!(wc.len(), 1);
            assert_eq!(wc[0].type_name, "T");
            assert_eq!(wc[0].bounds.len(), 2);
            assert_eq!(wc[0].bounds[0].as_str(), "mod1::Printable");
            assert_eq!(wc[0].bounds[1].as_str(), "mod2::Serializable");
        } else {
            panic!("Expected Function");
        }
    }
}
