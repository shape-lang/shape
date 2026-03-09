//! Integration tests for the Type System Overhaul.
//!
//! Tests compile and run Shape source code to verify:
//! - Generic type preservation through Vec/Table method chains
//! - Queryable trait compilation and dispatch
//! - Compiler heuristic elimination (MethodTable-driven type queries)
//! - Parser multi-generic support

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

// =============================================================================
// SECTION C: Parser multi-generic tests
// =============================================================================

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

#[test]
fn test_content_chart_from_table_value() {
    // Test that c"{data: chart(bar), x(month), y(sales)}" creates a Chart ContentNode
    let source = r#"
type SalesRecord { month: int, sales: int }
let data = [
    SalesRecord { month: 1, sales: 42 },
    SalesRecord { month: 2, sales: 58 },
    SalesRecord { month: 3, sales: 65 }
]
c"{data: chart(bar), x(month), y(sales)}"
"#;
    let result = compile_and_execute(source).unwrap();
    let content = result.as_content().expect("expected Content value");
    match content {
        shape_value::content::ContentNode::Chart(spec) => {
            assert_eq!(spec.chart_type, shape_value::content::ChartType::Bar);
            assert_eq!(spec.x_label.as_deref(), Some("month"));
            let y_channels = spec.channels_by_name("y");
            assert_eq!(y_channels.len(), 1);
            assert_eq!(y_channels[0].label, "sales");
            assert_eq!(y_channels[0].values.len(), 3);
            // x values: 1, 2, 3 — y values: 42, 58, 65
            let x_ch = spec.channel("x").unwrap();
            assert_eq!(x_ch.values, vec![1.0, 2.0, 3.0]);
            assert_eq!(y_channels[0].values, vec![42.0, 58.0, 65.0]);
        }
        _ => panic!("expected Chart variant, got {:?}", content),
    }
}

#[test]
fn test_content_chart_from_table_multi_y() {
    // Test multiple y columns
    let source = r#"
type FinRecord { x: int, revenue: int, cost: int }
let data = [
    FinRecord { x: 1, revenue: 100, cost: 60 },
    FinRecord { x: 2, revenue: 120, cost: 70 }
]
c"{data: chart(line), x(x), y(revenue, cost)}"
"#;
    let result = compile_and_execute(source).unwrap();
    let content = result.as_content().expect("expected Content value");
    match content {
        shape_value::content::ContentNode::Chart(spec) => {
            assert_eq!(spec.chart_type, shape_value::content::ChartType::Line);
            let y_channels = spec.channels_by_name("y");
            assert_eq!(y_channels.len(), 2);
            assert_eq!(y_channels[0].label, "revenue");
            assert_eq!(y_channels[1].label, "cost");
            let x_ch = spec.channel("x").unwrap();
            assert_eq!(x_ch.values, vec![1.0, 2.0]);
            assert_eq!(y_channels[0].values, vec![100.0, 120.0]);
            assert_eq!(y_channels[1].values, vec![60.0, 70.0]);
        }
        _ => panic!("expected Chart variant, got {:?}", content),
    }
}

// ===== Table Row Literal Tests =====

#[test]
fn test_table_row_literal_basic() {
    let source = r#"
type Record { id: int, value: int, name: string }
let t: Table<Record> = [1, 100, "alpha"], [2, 200, "beta"], [3, 300, "gamma"]
t.count()
"#;
    let result = compile_and_execute(source).expect("should compile and run");
    // count() returns the number of rows
    assert_eq!(result.as_i64().or(result.as_f64().map(|f| f as i64)), Some(3));
}

#[test]
fn test_table_row_literal_filter() {
    let source = r#"
type SalesRow { month: int, revenue: int }
let t: Table<SalesRow> = [1, 42], [2, 58], [3, 65], [4, 51]
let filtered = t.filter(|row| row.revenue > 50)
filtered.count()
"#;
    let result = compile_and_execute(source).expect("should compile and run");
    // Rows with revenue > 50: month=2(58), month=3(65), month=4(51) → 3 rows
    assert_eq!(result.as_i64().or(result.as_f64().map(|f| f as i64)), Some(3));
}

#[test]
fn test_table_row_literal_wrong_column_count() {
    let source = r#"
type Pair { a: int, b: int }
let t: Table<Pair> = [1, 2, 3], [4, 5, 6]
"#;
    let program = parse_program(source).unwrap();
    let mut compiler = BytecodeCompiler::new();
    compiler.set_source(source);
    let result = compiler.compile(&program);
    assert!(result.is_err(), "should error on column count mismatch");
    let err = format!("{:?}", result.unwrap_err());
    assert!(err.contains("3 values") && err.contains("2 fields"), "error should mention count mismatch: {}", err);
}

#[test]
fn test_table_row_literal_no_annotation() {
    let source = r#"
let t = [1, 2], [3, 4]
"#;
    let program = parse_program(source).unwrap();
    let mut compiler = BytecodeCompiler::new();
    compiler.set_source(source);
    let result = compiler.compile(&program);
    assert!(result.is_err(), "should error without Table<T> annotation");
    let err = format!("{:?}", result.unwrap_err());
    assert!(err.contains("Table<T>"), "error should mention Table<T>: {}", err);
}

#[test]
fn test_table_row_literal_chart() {
    // Integration test: table row literal → chart format spec
    let source = r#"
type MonthlySales { month: int, revenue: number, profit: number }

let data: Table<MonthlySales> =
    [1, 42.0, 18.0],
    [2, 58.0, 25.0],
    [3, 65.0, 31.0],
    [4, 51.0, 22.0],
    [5, 73.0, 35.0],
    [6, 89.0, 42.0]

c"{data: chart(bar), x(month), y(revenue, profit)}"
"#;
    let result = compile_and_execute(source).unwrap();
    let content = result.as_content().expect("expected Content value");
    match content {
        shape_value::content::ContentNode::Chart(spec) => {
            assert_eq!(spec.chart_type, shape_value::content::ChartType::Bar);
            assert_eq!(spec.x_label.as_deref(), Some("month"));
            let y_channels = spec.channels_by_name("y");
            assert_eq!(y_channels.len(), 2);
            assert_eq!(y_channels[0].label, "revenue");
            assert_eq!(y_channels[1].label, "profit");
            assert_eq!(y_channels[0].values.len(), 6);
            let x_ch = spec.channel("x").unwrap();
            assert_eq!(x_ch.values[0], 1.0);
            assert_eq!(x_ch.values[5], 6.0);
            assert_eq!(y_channels[0].values[0], 42.0);
            assert_eq!(y_channels[0].values[5], 89.0);
            assert_eq!(y_channels[1].values[0], 18.0);
            assert_eq!(y_channels[1].values[5], 42.0);
        }
        _ => panic!("expected Chart variant, got {:?}", content),
    }
}
