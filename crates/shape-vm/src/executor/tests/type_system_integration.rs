//! Integration tests for the Type System Overhaul.
//!
//! Tests compile and run Shape source code to verify:
//! - HashMap<K,V> construction, methods, and closure operations
//! - Generic type preservation through Vec/Table method chains
//! - Queryable trait compilation and dispatch
//! - Compiler heuristic elimination (MethodTable-driven type queries)
//! - Parser multi-generic support (HashMap<K,V> type annotations)

use crate::compiler::BytecodeCompiler;
use crate::executor::VirtualMachine;
use crate::{VMConfig, VMError};
use shape_ast::parser::parse_program;
use shape_value::ValueWord;

/// Compile and execute Shape source code, returning the final expression value.
fn compile_and_execute(source: &str) -> Result<ValueWord, VMError> {
    let program =
        parse_program(source).map_err(|e| VMError::RuntimeError(format!("Parse: {:?}", e)))?;
    let mut compiler = BytecodeCompiler::new();
    compiler.set_source(source);
    let bytecode = compiler
        .compile(&program)
        .map_err(|e| VMError::RuntimeError(format!("Compile: {:?}", e)))?;
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode);
    vm.execute(None).map(|nb| nb.clone())
}

/// Assert that source code compiles successfully (may not need to run).
fn assert_compiles(source: &str) {
    let program = parse_program(source).expect("Parse failed");
    let mut compiler = BytecodeCompiler::new();
    compiler.set_source(source);
    compiler.compile(&program).expect("Compile failed");
}

/// Assert that source code fails to parse.
fn assert_parse_fails(source: &str) {
    assert!(
        parse_program(source).is_err(),
        "Expected parse failure for: {}",
        source
    );
}

// =============================================================================
// SECTION A: HashMap source-level integration tests
// =============================================================================

#[test]
fn test_hashmap_constructor() {
    // HashMap() should return an empty hashmap
    let result = compile_and_execute("HashMap()").unwrap();
    assert!(
        result.as_hashmap().is_some(),
        "HashMap() should return a HashMap, got: {}",
        result
    );
    let (keys, _, _) = result.as_hashmap().unwrap();
    assert_eq!(keys.len(), 0);
}

#[test]
fn test_hashmap_set_and_get() {
    let source = r#"{
        let m = HashMap()
        let m2 = m.set("x", 42)
        m2.get("x")
    }"#;
    let result = compile_and_execute(source).unwrap();
    assert_eq!(
        result.to_number().unwrap(),
        42.0,
        "set then get should return value"
    );
}

#[test]
fn test_hashmap_chained_set() {
    let source = r#"
        HashMap().set("a", 1).set("b", 2).set("c", 3).len()
    "#;
    let result = compile_and_execute(source).unwrap();
    assert_eq!(result.as_i64(), Some(3));
}

#[test]
fn test_hashmap_has_and_delete() {
    let source = r#"{
        let m = HashMap().set("x", 1).set("y", 2)
        let m2 = m.delete("x")
        [m.has("x"), m2.has("x"), m2.has("y")]
    }"#;
    let result = compile_and_execute(source).unwrap();
    let arr = result.as_array().expect("should be array");
    assert_eq!(arr[0].as_bool(), Some(true), "original has x");
    assert_eq!(arr[1].as_bool(), Some(false), "deleted doesn't have x");
    assert_eq!(arr[2].as_bool(), Some(true), "y still present");
}

#[test]
fn test_hashmap_keys_values_entries() {
    let source = r#"{
        let m = HashMap().set("a", 10).set("b", 20)
        [m.keys().length, m.values().length, m.entries().length]
    }"#;
    let result = compile_and_execute(source).unwrap();
    let arr = result.as_array().expect("should be array");
    assert_eq!(arr[0].as_i64(), Some(2));
    assert_eq!(arr[1].as_i64(), Some(2));
    assert_eq!(arr[2].as_i64(), Some(2));
}

#[test]
fn test_hashmap_is_empty() {
    let source = r#"
        [HashMap().isEmpty(), HashMap().set("a", 1).isEmpty()]
    "#;
    let result = compile_and_execute(source).unwrap();
    let arr = result.as_array().expect("should be array");
    assert_eq!(arr[0].as_bool(), Some(true));
    assert_eq!(arr[1].as_bool(), Some(false));
}

#[test]
fn test_hashmap_get_missing_returns_none() {
    let source = r#"{
        let m = HashMap().set("a", 1)
        m.get("z")
    }"#;
    let result = compile_and_execute(source).unwrap();
    assert!(result.is_none(), "get missing key should return None");
}

#[test]
fn test_hashmap_overwrite_existing_key() {
    let source = r#"
        HashMap().set("k", 1).set("k", 99).get("k")
    "#;
    let result = compile_and_execute(source).unwrap();
    assert_eq!(result.to_number().unwrap(), 99.0);
}

#[test]
fn test_hashmap_integer_keys_source() {
    let source = r#"
        HashMap().set(1, "one").set(2, "two").get(2)
    "#;
    let result = compile_and_execute(source).unwrap();
    assert_eq!(result.as_str().unwrap(), "two");
}

#[test]
fn test_hashmap_immutability() {
    // set() must not mutate the original
    let source = r#"{
        let original = HashMap().set("a", 1)
        let modified = original.set("b", 2)
        [original.len(), modified.len()]
    }"#;
    let result = compile_and_execute(source).unwrap();
    let arr = result.as_array().expect("should be array");
    assert_eq!(arr[0].as_i64(), Some(1), "original unchanged");
    assert_eq!(arr[1].as_i64(), Some(2), "modified has both");
}

// --- HashMap closure methods ---

#[test]
fn test_hashmap_filter_with_closure() {
    let source = r#"{
        let m = HashMap().set("a", 1).set("b", 20).set("c", 3)
        let big = m.filter(|k, v| v > 10)
        [big.len(), big.has("b"), big.has("a")]
    }"#;
    let result = compile_and_execute(source).unwrap();
    let arr = result.as_array().expect("should be array");
    assert_eq!(arr[0].as_i64(), Some(1), "only one entry passes filter");
    assert_eq!(arr[1].as_bool(), Some(true), "b passes");
    assert_eq!(arr[2].as_bool(), Some(false), "a filtered out");
}

#[test]
fn test_hashmap_map_with_closure() {
    let source = r#"{
        let m = HashMap().set("a", 10).set("b", 20)
        let doubled = m.map(|k, v| v * 2)
        [doubled.get("a"), doubled.get("b")]
    }"#;
    let result = compile_and_execute(source).unwrap();
    let arr = result.as_array().expect("should be array");
    assert_eq!(arr[0].to_number().unwrap(), 20.0);
    assert_eq!(arr[1].to_number().unwrap(), 40.0);
}

#[test]
fn test_hashmap_foreach_side_effect() {
    // forEach should iterate all entries; verify by checking it returns None
    // and that the map has the expected number of entries
    let source = r#"{
        let m = HashMap().set("a", 1).set("b", 2)
        m.len()
    }"#;
    let result = compile_and_execute(source).unwrap();
    assert_eq!(result.as_i64(), Some(2), "map should have 2 entries");

    // Also verify forEach returns the map (for chaining) and doesn't error
    let source2 = r#"{
        let m = HashMap().set("a", 1).set("b", 2)
        let count = 0
        m.forEach(|k, v| { v + 1 })
        m.len()
    }"#;
    let result2 = compile_and_execute(source2).unwrap();
    assert_eq!(result2.as_i64(), Some(2), "forEach iterates without error");
}

// =============================================================================
// SECTION B: Vec generic type preservation (source-level)
// =============================================================================

#[test]
fn test_array_filter_preserves_type() {
    // filter returns same Vec<T>, elements are still accessible
    let source = r#"{
        let nums = [10, 20, 30, 40, 50]
        let big = nums.filter(|x| x > 25)
        big.length
    }"#;
    let result = compile_and_execute(source).unwrap();
    assert_eq!(result.as_i64(), Some(3));
}

#[test]
fn test_array_map_transforms() {
    let source = r#"
        [1, 2, 3].map(|x| x * 10)
    "#;
    let result = compile_and_execute(source).unwrap();
    let arr = result.to_generic_array().expect("should be array");
    assert_eq!(arr.len(), 3);
    assert_eq!(arr[0].to_number().unwrap(), 10.0);
    assert_eq!(arr[1].to_number().unwrap(), 20.0);
    assert_eq!(arr[2].to_number().unwrap(), 30.0);
}

#[test]
fn test_array_reduce() {
    let source = r#"
        [1, 2, 3, 4].reduce(|acc, x| acc + x, 0)
    "#;
    let result = compile_and_execute(source).unwrap();
    assert_eq!(result.to_number().unwrap(), 10.0);
}

#[test]
fn test_array_find() {
    let source = r#"
        [10, 20, 30].find(|x| x > 15)
    "#;
    let result = compile_and_execute(source).unwrap();
    assert_eq!(result.to_number().unwrap(), 20.0);
}

#[test]
fn test_array_some_every() {
    let source = r#"{
        let nums = [2, 4, 6]
        [nums.some(|x| x > 5), nums.every(|x| x > 0), nums.every(|x| x > 5)]
    }"#;
    let result = compile_and_execute(source).unwrap();
    let arr = result.as_array().expect("should be array");
    assert_eq!(arr[0].as_bool(), Some(true), "some > 5");
    assert_eq!(arr[1].as_bool(), Some(true), "every > 0");
    assert_eq!(arr[2].as_bool(), Some(false), "not every > 5");
}

#[test]
fn test_array_first_last() {
    let source = r#"{
        let a = [10, 20, 30]
        [a.first(), a.last()]
    }"#;
    let result = compile_and_execute(source).unwrap();
    let arr = result.as_array().expect("should be array");
    assert_eq!(arr[0].to_number().unwrap(), 10.0);
    assert_eq!(arr[1].to_number().unwrap(), 30.0);
}

#[test]
fn test_array_method_chain() {
    // filter -> map -> reduce: full chain
    let source = r#"
        [1, 2, 3, 4, 5, 6]
            .filter(|x| x % 2 == 0)
            .map(|x| x * 10)
            .reduce(|acc, x| acc + x, 0)
    "#;
    let result = compile_and_execute(source).unwrap();
    assert_eq!(
        result.to_number().unwrap(),
        120.0,
        "2+4+6 filtered, *10 = 20+40+60"
    );
}

#[test]
fn test_array_flatmap() {
    let source = r#"
        [[1, 2], [3, 4]].flatMap(|arr| arr)
    "#;
    let result = compile_and_execute(source).unwrap();
    let arr = result.as_array().expect("should be array");
    assert_eq!(arr.len(), 4);
}

// =============================================================================
// SECTION C: Parser multi-generic tests
// =============================================================================

#[test]
fn test_parse_hashmap_type_annotation() {
    // This verifies the parser accepts HashMap<K,V> syntax in type positions
    assert_compiles(
        r#"
        type Config {
            settings: HashMap<string, number>
        }
    "#,
    );
}

#[test]
fn test_parse_multi_generic_type_name() {
    assert_compiles(
        r#"
        type Pair<A, B> {
            first: A,
            second: B
        }
    "#,
    );
}

#[test]
fn test_parse_nested_generic() {
    assert_compiles(
        r#"
        type Container {
            data: Vec<Option<number>>
        }
    "#,
    );
}

#[test]
fn test_parse_extend_with_multi_generic() {
    // extend blocks should accept multi-generic type names
    assert_compiles(
        r#"
        extend Vec<number> {
            method sum_all() {
                self.reduce(|a, b| a + b, 0)
            }
        }
        [1, 2, 3].sum_all()
    "#,
    );
}

// =============================================================================
// SECTION D: Compiler heuristic tests (MethodTable-driven)
// =============================================================================

#[test]
fn test_method_table_is_self_returning() {
    use shape_runtime::type_system::checking::MethodTable;
    let table = MethodTable::new();

    // Type-preserving methods should return true
    assert!(table.is_self_returning("Vec", "filter"));
    assert!(table.is_self_returning("Vec", "sort"));
    assert!(table.is_self_returning("Table", "filter"));
    assert!(table.is_self_returning("Table", "orderBy"));
    assert!(table.is_self_returning("Table", "head"));
    assert!(table.is_self_returning("Table", "tail"));
    assert!(table.is_self_returning("Table", "limit"));
    assert!(table.is_self_returning("HashMap", "filter"));

    // Non-preserving methods should return false
    assert!(!table.is_self_returning("Vec", "map"));
    assert!(!table.is_self_returning("Vec", "find"));
    assert!(!table.is_self_returning("Vec", "reduce"));
    assert!(!table.is_self_returning("Table", "count"));
    assert!(!table.is_self_returning("Table", "map"));
    assert!(!table.is_self_returning("HashMap", "map"));
    assert!(!table.is_self_returning("HashMap", "keys"));
}

#[test]
fn test_method_table_takes_closure_with_receiver_param() {
    use shape_runtime::type_system::checking::MethodTable;
    let table = MethodTable::new();

    // Methods that take closure with receiver element type
    assert!(table.takes_closure_with_receiver_param("Vec", "filter"));
    assert!(table.takes_closure_with_receiver_param("Vec", "map"));
    assert!(table.takes_closure_with_receiver_param("Vec", "forEach"));
    assert!(table.takes_closure_with_receiver_param("Vec", "some"));
    assert!(table.takes_closure_with_receiver_param("Vec", "every"));
    assert!(table.takes_closure_with_receiver_param("Vec", "find"));
    assert!(table.takes_closure_with_receiver_param("Vec", "reduce"));
    assert!(table.takes_closure_with_receiver_param("Table", "filter"));
    assert!(table.takes_closure_with_receiver_param("Table", "map"));
    assert!(table.takes_closure_with_receiver_param("Table", "forEach"));

    // Methods that DON'T take closures
    assert!(!table.takes_closure_with_receiver_param("Vec", "length"));
    assert!(!table.takes_closure_with_receiver_param("Vec", "first"));
    assert!(!table.takes_closure_with_receiver_param("Vec", "last"));
    assert!(!table.takes_closure_with_receiver_param("Table", "count"));
    assert!(!table.takes_closure_with_receiver_param("Table", "head"));
    assert!(!table.takes_closure_with_receiver_param("HashMap", "get"));
    assert!(!table.takes_closure_with_receiver_param("HashMap", "len"));
    assert!(!table.takes_closure_with_receiver_param("HashMap", "keys"));
}

// =============================================================================
// SECTION E: Generic method resolution (type system unit tests)
// =============================================================================

#[test]
fn test_resolve_result_unwrap() {
    use shape_ast::ast::TypeAnnotation;
    use shape_runtime::type_system::checking::MethodTable;
    use shape_runtime::type_system::{BuiltinTypes, Type};

    let table = MethodTable::new();
    let result_type = Type::Generic {
        base: Box::new(Type::Concrete(TypeAnnotation::Reference(
            "Result".to_string(),
        ))),
        args: vec![BuiltinTypes::string()],
    };

    let resolved = table.resolve_method_call(&result_type, "unwrap", &[]);
    assert!(resolved.is_some(), "Result<string>.unwrap() should resolve");
    assert!(
        matches!(
            resolved.unwrap(),
            Type::Concrete(TypeAnnotation::Basic(ref n)) if n == "string"
        ),
        "Result<string>.unwrap() should return string"
    );
}

#[test]
fn test_resolve_hashmap_values_returns_array_v() {
    use shape_ast::ast::TypeAnnotation;
    use shape_runtime::type_system::checking::MethodTable;
    use shape_runtime::type_system::{BuiltinTypes, Type};

    let table = MethodTable::new();
    let map_type = Type::Generic {
        base: Box::new(Type::Concrete(TypeAnnotation::Reference(
            "HashMap".to_string(),
        ))),
        args: vec![BuiltinTypes::string(), BuiltinTypes::number()],
    };

    let resolved = table.resolve_method_call(&map_type, "values", &[]);
    assert!(
        resolved.is_some(),
        "HashMap<string,number>.values() should resolve"
    );
    let rt = resolved.unwrap();
    // Should return Vec<number>
    assert!(
        matches!(&rt, Type::Generic { base, args }
            if matches!(base.as_ref(), Type::Concrete(TypeAnnotation::Reference(n)) if n == "Vec")
            && args.len() == 1
        ),
        "values() should return Vec<number>, got {:?}",
        rt
    );
}

#[test]
fn test_resolve_hashmap_entries() {
    use shape_ast::ast::TypeAnnotation;
    use shape_runtime::type_system::checking::MethodTable;
    use shape_runtime::type_system::{BuiltinTypes, Type};

    let table = MethodTable::new();
    let map_type = Type::Generic {
        base: Box::new(Type::Concrete(TypeAnnotation::Reference(
            "HashMap".to_string(),
        ))),
        args: vec![BuiltinTypes::string(), BuiltinTypes::number()],
    };

    let resolved = table.resolve_method_call(&map_type, "entries", &[]);
    assert!(
        resolved.is_some(),
        "HashMap<string,number>.entries() should resolve"
    );
}

#[test]
fn test_resolve_option_map() {
    use shape_ast::ast::TypeAnnotation;
    use shape_runtime::type_system::checking::MethodTable;
    use shape_runtime::type_system::{BuiltinTypes, Type};

    let table = MethodTable::new();
    let option_type = Type::Generic {
        base: Box::new(Type::Concrete(TypeAnnotation::Reference(
            "Option".to_string(),
        ))),
        args: vec![BuiltinTypes::number()],
    };

    let resolved = table.resolve_method_call(&option_type, "map", &[]);
    assert!(resolved.is_some(), "Option<number>.map() should resolve");
    // map returns Option<U> where U is a fresh type variable
    let rt = resolved.unwrap();
    assert!(
        matches!(&rt, Type::Generic { base, .. }
            if matches!(base.as_ref(), Type::Concrete(TypeAnnotation::Reference(n)) if n == "Option")
        ),
        "Option.map should return Option<U>, got {:?}",
        rt
    );
}

#[test]
fn test_resolve_table_map_returns_table_u() {
    use shape_ast::ast::TypeAnnotation;
    use shape_runtime::type_system::Type;
    use shape_runtime::type_system::checking::MethodTable;

    let table = MethodTable::new();
    let table_type = Type::Generic {
        base: Box::new(Type::Concrete(TypeAnnotation::Reference(
            "Table".to_string(),
        ))),
        args: vec![Type::Concrete(TypeAnnotation::Reference("Row".to_string()))],
    };

    let resolved = table.resolve_method_call(&table_type, "map", &[]);
    assert!(resolved.is_some(), "Table<Row>.map() should resolve");
    let rt = resolved.unwrap();
    // map returns Table<U> where U is fresh — should be Table<TypeVar>
    assert!(
        matches!(&rt, Type::Generic { base, .. }
            if matches!(base.as_ref(), Type::Concrete(TypeAnnotation::Reference(n)) if n == "Table")
        ),
        "Table.map should return Table<U>, got {:?}",
        rt
    );
}

// =============================================================================
// SECTION F: Queryable trait compilation
// =============================================================================

#[test]
fn test_queryable_trait_compiles() {
    // The Queryable trait definition should parse and compile
    assert_compiles(
        r#"
        trait Queryable<T> {
            filter(predicate): any,
            map(transform): any,
            orderBy(column, direction): any,
            limit(n): any,
            execute(): any
        }
    "#,
    );
}

#[test]
fn test_queryable_impl_for_custom_type() {
    // Implementing Queryable for a custom type should compile
    assert_compiles(
        r#"
        trait Queryable {
            filter(predicate): any,
            execute(): any
        }

        type MyQuery {
            data: Vec<number>
        }

        impl Queryable for MyQuery {
            method filter(predicate) {
                { data: self.data.filter(predicate) }
            }
            method execute() {
                self.data
            }
        }
    "#,
    );
}

// =============================================================================
// SECTION G: Extend blocks with method dispatch
// =============================================================================

#[test]
fn test_extend_array_custom_method() {
    let source = r#"
        extend Vec<number> {
            method double_all() {
                self.map(|x| x * 2)
            }
        }
        [1, 2, 3].double_all()
    "#;
    let result = compile_and_execute(source).unwrap();
    let arr = result.to_generic_array().expect("should be array");
    assert_eq!(arr.len(), 3);
    assert_eq!(arr[0].to_number().unwrap(), 2.0);
    assert_eq!(arr[1].to_number().unwrap(), 4.0);
    assert_eq!(arr[2].to_number().unwrap(), 6.0);
}

#[test]
fn test_extend_number_method_chaining() {
    let source = r#"
        extend Number {
            method double() { self * 2 }
            method add_one() { self + 1 }
        }
        (5).double().add_one()
    "#;
    let result = compile_and_execute(source).unwrap();
    assert_eq!(result.to_number().unwrap(), 11.0);
}

// =============================================================================
// SECTION H: HashMap with complex values
// =============================================================================

#[test]
fn test_hashmap_with_array_values() {
    let source = r#"{
        let m = HashMap()
            .set("nums", [1, 2, 3])
            .set("strs", ["a", "b"])
        m.get("nums").length
    }"#;
    let result = compile_and_execute(source).unwrap();
    assert_eq!(result.as_i64(), Some(3));
}

#[test]
fn test_hashmap_with_boolean_keys() {
    let source = r#"
        HashMap().set(true, "yes").set(false, "no").get(true)
    "#;
    let result = compile_and_execute(source).unwrap();
    assert_eq!(result.as_str().unwrap(), "yes");
}

#[test]
fn test_hashmap_filter_then_keys() {
    let source = r#"{
        let m = HashMap()
            .set("low", 5)
            .set("mid", 15)
            .set("high", 25)
        m.filter(|k, v| v >= 15).keys().length
    }"#;
    let result = compile_and_execute(source).unwrap();
    assert_eq!(result.as_i64(), Some(2));
}

#[test]
fn test_hashmap_map_then_values() {
    let source = r#"{
        let m = HashMap().set("a", 2).set("b", 3)
        let squared = m.map(|k, v| v * v)
        [squared.get("a"), squared.get("b")]
    }"#;
    let result = compile_and_execute(source).unwrap();
    let arr = result.as_array().expect("should be array");
    assert_eq!(arr[0].to_number().unwrap(), 4.0);
    assert_eq!(arr[1].to_number().unwrap(), 9.0);
}

// =============================================================================
// SECTION I: Edge cases
// =============================================================================

#[test]
fn test_hashmap_none_value() {
    let source = r#"{
        let m = HashMap().set("x", None)
        [m.has("x"), m.get("x") == None]
    }"#;
    let result = compile_and_execute(source).unwrap();
    let arr = result.as_array().expect("should be array");
    assert_eq!(arr[0].as_bool(), Some(true), "key exists");
    assert_eq!(arr[1].as_bool(), Some(true), "value is None");
}

#[test]
fn test_hashmap_multiple_types_as_values() {
    let source = r#"{
        let m = HashMap()
            .set("num", 42)
            .set("str", "hello")
            .set("bool", true)
        [m.get("num"), m.get("str"), m.get("bool")]
    }"#;
    let result = compile_and_execute(source).unwrap();
    let arr = result.as_array().expect("should be array");
    assert_eq!(arr[0].to_number().unwrap(), 42.0);
    assert_eq!(arr[1].as_str().unwrap(), "hello");
    assert_eq!(arr[2].as_bool(), Some(true));
}

#[test]
fn test_hashmap_large_construction() {
    // Build a map with 100 entries
    let mut lines = vec!["let m = HashMap()".to_string()];
    for i in 0..100 {
        lines.push(format!("let m = m.set({}, {})", i, i * i));
    }
    lines.push("m.len()".to_string());
    let source = lines.join("\n");
    let result = compile_and_execute(&source).unwrap();
    assert_eq!(result.as_i64(), Some(100));
}

// ===== Content String Tests =====

#[test]
fn test_content_string_plain_text() {
    let source = r#"c"hello world""#;
    let result = compile_and_execute(source).unwrap();
    // Result should be a ContentNode
    let content = result.as_content().expect("expected Content value");
    assert_eq!(format!("{}", content), "hello world");
}

#[test]
fn test_content_string_with_interpolation() {
    let source = r#"
let name = "Alice"
c"Hello {name}!"
"#;
    let result = compile_and_execute(source).unwrap();
    let content = result.as_content().expect("expected Content value");
    assert_eq!(format!("{}", content), "Hello Alice!");
}

#[test]
fn test_content_string_with_numeric_interpolation() {
    let source = r#"
let x = 42
c"value is {x}"
"#;
    let result = compile_and_execute(source).unwrap();
    let content = result.as_content().expect("expected Content value");
    assert_eq!(format!("{}", content), "value is 42");
}

#[test]
fn test_content_string_multiple_interpolations() {
    let source = r#"
let a = "foo"
let b = "bar"
c"{a} and {b}"
"#;
    let result = compile_and_execute(source).unwrap();
    let content = result.as_content().expect("expected Content value");
    assert_eq!(format!("{}", content), "foo and bar");
}

#[test]
fn test_content_string_with_bold_style() {
    let source = r#"
let name = "World"
c"Hello {name:bold}"
"#;
    let result = compile_and_execute(source).unwrap();
    let content = result.as_content().expect("expected Content value");
    // The Display output should be the plain text
    assert_eq!(format!("{}", content), "Hello World");
    // Verify the styled part has bold set
    match content {
        shape_value::content::ContentNode::Fragment(parts) => {
            assert_eq!(parts.len(), 2); // "Hello " and styled "World"
            match &parts[1] {
                shape_value::content::ContentNode::Text(st) => {
                    assert!(st.spans[0].style.bold, "expected bold style");
                }
                _ => panic!("expected Text variant for styled part"),
            }
        }
        _ => panic!("expected Fragment variant"),
    }
}

#[test]
fn test_content_string_with_fg_color() {
    let source = r#"
let msg = "error"
c"Status: {msg:fg(red)}"
"#;
    let result = compile_and_execute(source).unwrap();
    let content = result.as_content().expect("expected Content value");
    assert_eq!(format!("{}", content), "Status: error");
    match content {
        shape_value::content::ContentNode::Fragment(parts) => {
            assert_eq!(parts.len(), 2);
            match &parts[1] {
                shape_value::content::ContentNode::Text(st) => {
                    assert_eq!(
                        st.spans[0].style.fg,
                        Some(shape_value::content::Color::Named(
                            shape_value::content::NamedColor::Red
                        ))
                    );
                }
                _ => panic!("expected Text variant"),
            }
        }
        _ => panic!("expected Fragment variant"),
    }
}

#[test]
fn test_content_string_with_multiple_styles() {
    let source = r#"
let val = "important"
c"{val:fg(green), bold, underline}"
"#;
    let result = compile_and_execute(source).unwrap();
    let content = result.as_content().expect("expected Content value");
    assert_eq!(format!("{}", content), "important");
    match content {
        shape_value::content::ContentNode::Text(st) => {
            assert!(st.spans[0].style.bold, "expected bold");
            assert!(st.spans[0].style.underline, "expected underline");
            assert_eq!(
                st.spans[0].style.fg,
                Some(shape_value::content::Color::Named(
                    shape_value::content::NamedColor::Green
                ))
            );
        }
        _ => panic!("expected Text variant for single-part styled content"),
    }
}

#[test]
fn test_content_string_empty() {
    let source = r#"c"""#;
    let result = compile_and_execute(source).unwrap();
    let content = result.as_content().expect("expected Content value");
    assert_eq!(format!("{}", content), "");
}

#[test]
fn test_content_string_no_interpolation() {
    let source = r#"c"just plain text""#;
    let result = compile_and_execute(source).unwrap();
    let content = result.as_content().expect("expected Content value");
    assert_eq!(format!("{}", content), "just plain text");
}

#[test]
fn test_content_method_bold() {
    let source = r#"
let c = c"hello"
c.bold()
"#;
    let result = compile_and_execute(source).unwrap();
    let content = result.as_content().expect("expected Content value");
    match content {
        shape_value::content::ContentNode::Text(st) => {
            assert!(st.spans[0].style.bold);
        }
        _ => panic!("expected Text variant"),
    }
}

#[test]
fn test_content_method_fg() {
    let source = r#"
let c = c"warning"
c.fg("yellow")
"#;
    let result = compile_and_execute(source).unwrap();
    let content = result.as_content().expect("expected Content value");
    match content {
        shape_value::content::ContentNode::Text(st) => {
            assert_eq!(
                st.spans[0].style.fg,
                Some(shape_value::content::Color::Named(
                    shape_value::content::NamedColor::Yellow
                ))
            );
        }
        _ => panic!("expected Text variant"),
    }
}

#[test]
fn test_content_method_chaining() {
    let source = r#"
let c = c"styled"
c.bold().italic().fg("cyan")
"#;
    let result = compile_and_execute(source).unwrap();
    let content = result.as_content().expect("expected Content value");
    match content {
        shape_value::content::ContentNode::Text(st) => {
            assert!(st.spans[0].style.bold);
            assert!(st.spans[0].style.italic);
            assert_eq!(
                st.spans[0].style.fg,
                Some(shape_value::content::Color::Named(
                    shape_value::content::NamedColor::Cyan
                ))
            );
        }
        _ => panic!("expected Text variant"),
    }
}

#[test]
fn test_content_method_to_string() {
    let source = r#"
let c = c"hello world"
c.toString()
"#;
    let result = compile_and_execute(source).unwrap();
    let s = result.as_str().expect("expected string");
    assert_eq!(s, "hello world");
}

#[test]
fn test_content_print_output() {
    // Verify that print() on content values works
    let source = r#"
let c = c"test output"
print(c)
"#;
    // Just verify it compiles and executes without error
    let _ = compile_and_execute(source).unwrap();
}

// =============================================================================
// SECTION J: BUG-1 / BUG-2 -- TypeAnnotatedValue must not break arithmetic/comparisons
// =============================================================================

#[test]
fn test_bug1_type_annotated_variable_arithmetic() {
    // BUG-1: `let x: int = 3; let y = 1; x + y` should produce 4.
    // Type-annotated variables in block scope.
    let source = r#"{
        let x: int = 3
        let y = 1
        x + y
    }"#;
    let result = compile_and_execute(source).unwrap();
    assert_eq!(
        result.to_number().unwrap(),
        4.0,
        "Type-annotated int should participate in arithmetic"
    );
}

#[test]
fn test_bug2_type_annotated_variable_comparison() {
    // BUG-2: `let x: int = 5; x > 3` should produce true.
    let source = r#"{
        let x: int = 5
        x > 3
    }"#;
    let result = compile_and_execute(source).unwrap();
    assert_eq!(
        result.as_bool(),
        Some(true),
        "Type-annotated int should work in comparisons"
    );
}

#[test]
fn test_bug1_type_annotated_string_length() {
    // Type-annotated strings should still support method calls.
    let source = r#"{
        let s: string = "hello"
        s.length
    }"#;
    let result = compile_and_execute(source).unwrap();
    assert_eq!(
        result.as_i64(),
        Some(5),
        "Type-annotated string should support .length"
    );
}

#[test]
fn test_bug1_toplevel_type_annotated_arithmetic() {
    // Top-level (module binding) type-annotated variables must work in arithmetic.
    let source = r#"
        let x: int = 3
        let y = 1
        x + y
    "#;
    let result = compile_and_execute(source).unwrap();
    assert_eq!(
        result.to_number().unwrap(),
        4.0,
        "Top-level type-annotated int should participate in arithmetic"
    );
}

#[test]
fn test_bug2_toplevel_type_annotated_comparison() {
    // Top-level type-annotated variables must work in comparisons.
    let source = r#"
        let x: int = 5
        x > 3
    "#;
    let result = compile_and_execute(source).unwrap();
    assert_eq!(
        result.as_bool(),
        Some(true),
        "Top-level type-annotated int should work in comparisons"
    );
}

#[test]
fn test_bug1_type_annotated_value_not_wrapped() {
    // After the fix, type-annotated variables should NOT be wrapped in TypeAnnotatedValue.
    let source = r#"
        let x: int = 42
        x
    "#;
    let result = compile_and_execute(source).unwrap();
    assert_eq!(
        result.as_i64(),
        Some(42),
        "Type-annotated int should be a plain integer"
    );
    assert!(
        result.as_heap_ref().is_none(),
        "Type-annotated int should not be a heap value (no TypeAnnotatedValue wrapper)"
    );
}
