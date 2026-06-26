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
use shape_value::content::{ChartSpec, ChartType, ContentNode};
use shape_value::{HeapKind, KindedSlot, NativeKind};

/// Compile and execute Shape source code, returning the final expression value.
fn compile_and_execute(source: &str) -> Result<KindedSlot, VMError> {
    let program =
        parse_program(source).map_err(|e| VMError::RuntimeError(format!("Parse: {:?}", e)))?;
    let mut compiler = BytecodeCompiler::new();
    compiler.set_source(source);
    let bytecode = compiler
        .compile(&program)
        .map_err(|e| VMError::RuntimeError(format!("Compile: {:?}", e)))?;
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode);
    vm.execute(None)
}

fn assert_int_result(result: &KindedSlot, expected: i64, context: &str) {
    assert_eq!(result.kind(), shape_value::NativeKind::Int64, "{context}");
    assert_eq!(result.as_i64(), Some(expected), "{context}");
}

fn assert_numeric_result(result: &KindedSlot, expected: f64, context: &str) {
    let actual = result
        .as_f64()
        .or_else(|| result.as_i64().map(|v| v as f64))
        .expect(context);
    assert_eq!(actual, expected, "{context}");
}

fn content_result<'a>(result: &'a KindedSlot, context: &str) -> &'a ContentNode {
    assert_eq!(
        result.kind(),
        NativeKind::Ptr(HeapKind::Content),
        "{context}"
    );
    let bits = result.raw();
    assert_ne!(bits, 0, "{context}");
    // SAFETY: Ptr(Content) slots are Arc::into_raw(Arc<ContentNode>) carriers.
    unsafe { &*(bits as *const ContentNode) }
}

fn chart_result<'a>(result: &'a KindedSlot, context: &str) -> &'a ChartSpec {
    match content_result(result, context) {
        ContentNode::Chart(spec) => spec,
        other => panic!("{context}: expected chart content, got {other:?}"),
    }
}

fn assert_channel(spec: &ChartSpec, name: &str, label: &str, values: &[f64]) {
    let matches: Vec<_> = spec
        .channels_by_name(name)
        .into_iter()
        .filter(|channel| channel.label == label)
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one {name} channel labeled {label}, got {:?}",
        spec.channels
    );
    assert_eq!(matches[0].values, values);
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
// Methods are now registered from Shape stdlib, not at MethodTable::new().
// These tests manually register the methods they need to verify the
// MethodTable infrastructure still works correctly.
// =============================================================================

#[test]
fn test_method_table_is_self_returning() {
    use shape_runtime::type_system::checking::{MethodTable, TypeParamExpr};
    let mut table = MethodTable::new();
    table.register_user_generic_method("Vec", "filter", 0, vec![], TypeParamExpr::SelfType, vec![]);
    table.register_user_generic_method(
        "Vec",
        "map",
        1,
        vec![],
        TypeParamExpr::GenericContainer {
            name: "Vec".to_string(),
            args: vec![TypeParamExpr::MethodParam(0)],
        },
        vec![],
    );
    assert!(table.is_self_returning("Vec", "filter"));
    assert!(!table.is_self_returning("Vec", "map"));
}

#[test]
fn test_method_table_takes_closure_with_receiver_param() {
    use shape_runtime::type_system::checking::{MethodTable, TypeParamExpr};
    use shape_runtime::type_system::BuiltinTypes;
    let mut table = MethodTable::new();
    table.register_user_generic_method(
        "Vec",
        "filter",
        0,
        vec![TypeParamExpr::Function {
            params: vec![TypeParamExpr::ReceiverParam(0)],
            returns: Box::new(TypeParamExpr::Concrete(BuiltinTypes::boolean())),
        }],
        TypeParamExpr::SelfType,
        vec![],
    );
    assert!(table.takes_closure_with_receiver_param("Vec", "filter"));
    assert!(!table.takes_closure_with_receiver_param("Vec", "len"));
}

// =============================================================================
// SECTION E: Generic method resolution (type system unit tests)
// =============================================================================

#[test]
fn test_resolve_result_unwrap() {
    use shape_ast::ast::TypeAnnotation;
    use shape_runtime::type_system::checking::{MethodTable, TypeParamExpr};
    use shape_runtime::type_system::{BuiltinTypes, Type};

    let mut table = MethodTable::new();
    table.register_user_generic_method(
        "Result",
        "unwrap",
        0,
        vec![],
        TypeParamExpr::ReceiverParam(0),
        vec![],
    );

    let result_type = Type::Generic {
        base: Box::new(Type::Concrete(TypeAnnotation::Reference("Result".into()))),
        args: vec![BuiltinTypes::string()],
    };
    let mut tvgen = shape_runtime::type_system::TypeVarGen::new();
    let resolved = table.resolve_method_call(&result_type, "unwrap", &[], &mut tvgen);
    assert!(resolved.is_some(), "Result<string>.unwrap() should resolve");
    assert!(
        matches!(resolved.unwrap(), Type::Concrete(TypeAnnotation::Basic(ref n)) if n == "string"),
        "Result<string>.unwrap() should return string"
    );
}

#[test]
fn test_resolve_option_map() {
    use shape_ast::ast::TypeAnnotation;
    use shape_runtime::type_system::checking::{MethodTable, TypeParamExpr};
    use shape_runtime::type_system::{BuiltinTypes, Type};

    let mut table = MethodTable::new();
    table.register_user_generic_method(
        "Option",
        "map",
        1,
        vec![TypeParamExpr::Function {
            params: vec![TypeParamExpr::ReceiverParam(0)],
            returns: Box::new(TypeParamExpr::MethodParam(0)),
        }],
        TypeParamExpr::GenericContainer {
            name: "Option".to_string(),
            args: vec![TypeParamExpr::MethodParam(0)],
        },
        vec![],
    );

    let option_type = Type::Generic {
        base: Box::new(Type::Concrete(TypeAnnotation::Reference("Option".into()))),
        args: vec![BuiltinTypes::number()],
    };
    let mut tvgen = shape_runtime::type_system::TypeVarGen::new();
    let resolved = table.resolve_method_call(&option_type, "map", &[], &mut tvgen);
    assert!(resolved.is_some(), "Option<number>.map() should resolve");
    let rt = resolved.unwrap();
    assert!(
        matches!(&rt, Type::Generic { base, .. }
            if matches!(base.as_ref(), Type::Concrete(TypeAnnotation::Reference(n)) if n == "Option")),
        "Option.map should return Option<U>, got {:?}",
        rt
    );
}

#[test]
fn test_resolve_table_map_returns_table_u() {
    use shape_ast::ast::TypeAnnotation;
    use shape_runtime::type_system::checking::{MethodTable, TypeParamExpr};
    use shape_runtime::type_system::Type;

    let mut table = MethodTable::new();
    table.register_user_generic_method(
        "Table",
        "map",
        1,
        vec![TypeParamExpr::Function {
            params: vec![TypeParamExpr::ReceiverParam(0)],
            returns: Box::new(TypeParamExpr::MethodParam(0)),
        }],
        TypeParamExpr::GenericContainer {
            name: "Table".to_string(),
            args: vec![TypeParamExpr::MethodParam(0)],
        },
        vec![],
    );

    let table_type = Type::Generic {
        base: Box::new(Type::Concrete(TypeAnnotation::Reference("Table".into()))),
        args: vec![Type::Concrete(TypeAnnotation::Reference("Row".into()))],
    };
    let mut tvgen = shape_runtime::type_system::TypeVarGen::new();
    let resolved = table.resolve_method_call(&table_type, "map", &[], &mut tvgen);
    assert!(resolved.is_some(), "Table<Row>.map() should resolve");
    let rt = resolved.unwrap();
    assert!(
        matches!(&rt, Type::Generic { base, .. }
            if matches!(base.as_ref(), Type::Concrete(TypeAnnotation::Reference(n)) if n == "Table")),
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
            method filter(predicate) -> any
            method map(transform) -> any
            method orderBy(column, direction) -> any
            method limit(n) -> any
            method execute() -> any
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
            method filter(predicate) -> any
            method execute() -> any
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
        extend Vec {
            method item_count() -> int {
                self.len()
            }
        }

        [4, 8, 15].item_count()
    "#;
    let result = compile_and_execute(source).unwrap();
    assert_int_result(
        &result,
        3,
        "extend Vec custom method should dispatch with the typed array receiver",
    );
}

#[test]
fn test_extend_number_method_chaining() {
    let source = r#"
        extend Number {
            method add(n: int) -> number {
                self + n
            }

            method double() -> number {
                self * 2
            }
        }

        5.add(3).double()
    "#;
    let result = compile_and_execute(source).unwrap();
    assert_numeric_result(
        &result,
        16.0,
        "extend Number methods should remain chainable after returning number",
    );
}

// =============================================================================
// SECTION J: BUG-1 / BUG-2 -- TypeAnnotatedValue must not break arithmetic/comparisons
// =============================================================================

#[test]
fn test_bug1_type_annotated_variable_arithmetic() {
    // BUG-1: `let x: int = 3; let y = 1; x + y` should produce 4.
    let source = r#"{
        let x: int = 3
        let y = 1
        x + y
    }"#;
    let result = compile_and_execute(source).unwrap();
    assert_int_result(
        &result,
        4,
        "Type-annotated int should participate in arithmetic",
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
    // Top-level module-binding type-annotated variables must work in arithmetic.
    let source = r#"
        let x: int = 3
        let y = 1
        x + y
    "#;
    let result = compile_and_execute(source).unwrap();
    assert_int_result(
        &result,
        4,
        "Top-level type-annotated int should participate in arithmetic",
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
    let source = r#"
        let x: int = 42
        x
    "#;
    let result = compile_and_execute(source).unwrap();
    assert_int_result(&result, 42, "Type-annotated int should be a plain integer");
}

#[test]
fn test_content_chart_from_table_value() {
    let source = r#"
type SalesRecord { month: int, sales: int }
let data = [
    SalesRecord { month: 1, sales: 42 },
    SalesRecord { month: 2, sales: 58 },
    SalesRecord { month: 3, sales: 65 }
]
f"{data: chart(bar), x(month), y(sales)}"
"#;
    let result = compile_and_execute(source).unwrap();
    let spec = chart_result(
        &result,
        "chart-formatted typed-record arrays should return chart content",
    );
    assert_eq!(spec.chart_type, ChartType::Bar);
    assert_eq!(spec.x_label.as_deref(), Some("month"));
    assert_channel(spec, "x", "month", &[1.0, 2.0, 3.0]);
    assert_channel(spec, "y", "sales", &[42.0, 58.0, 65.0]);
}

#[test]
fn test_content_chart_from_table_multi_y() {
    let source = r#"
type FinRecord { x: int, revenue: int, cost: int }
let data = [
    FinRecord { x: 1, revenue: 100, cost: 60 },
    FinRecord { x: 2, revenue: 120, cost: 70 }
]
f"{data: chart(line), x(x), y(revenue, cost)}"
"#;
    let result = compile_and_execute(source).unwrap();
    let spec = chart_result(
        &result,
        "chart-formatted typed-record arrays should support multiple y channels",
    );
    assert_eq!(spec.chart_type, ChartType::Line);
    assert_eq!(spec.x_label.as_deref(), Some("x"));
    assert_channel(spec, "x", "x", &[1.0, 2.0]);
    assert_channel(spec, "y", "revenue", &[100.0, 120.0]);
    assert_channel(spec, "y", "cost", &[60.0, 70.0]);
}

#[test]
fn test_content_chart_rejects_decimal_field_projection() {
    let typed_object_source = r#"
type Quote { day: int, price: decimal }
let data = [
    Quote { day: 1, price: 1.5D },
    Quote { day: 2, price: 2.5D }
]
f"{data: chart(line), x(day), y(price)}"
"#;
    let err = compile_and_execute(typed_object_source)
        .expect_err("decimal typed-object chart projection must reject at schema validation");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("Quote.price") && msg.contains("field type decimal"),
        "decimal typed-object rejection should cite schema field type, got: {msg}"
    );
    assert!(
        !msg.contains("got kind") && !msg.contains("Arrow type"),
        "decimal typed-object rejection should happen before carrier projection, got: {msg}"
    );

    let table_source = r#"
type Quote { day: int, price: decimal }
let data: Table<Quote> = [1, 10], [2, 20]
f"{data: chart(line), x(day), y(price)}"
"#;
    let err = compile_and_execute(table_source)
        .expect_err("decimal Table<T> chart projection must reject at schema validation");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("Quote.price") && msg.contains("field type decimal"),
        "decimal table rejection should cite schema field type, got: {msg}"
    );
    assert!(
        !msg.contains("got kind") && !msg.contains("Arrow type"),
        "decimal table rejection should happen before carrier projection, got: {msg}"
    );
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
    assert_eq!(
        result.as_i64().or(result.as_f64().map(|f| f as i64)),
        Some(3)
    );
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
    assert_eq!(
        result.as_i64().or(result.as_f64().map(|f| f as i64)),
        Some(3)
    );
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
    assert!(
        err.contains("3 values") && err.contains("2 fields"),
        "error should mention count mismatch: {}",
        err
    );
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
    assert!(
        err.contains("Table<T>"),
        "error should mention Table<T>: {}",
        err
    );
}

#[test]
fn test_table_row_literal_chart() {
    let source = r#"
type MonthlySales { month: int, revenue: number, profit: number }

let data: Table<MonthlySales> =
    [1, 42.0, 18.0],
    [2, 58.0, 25.0],
    [3, 65.0, 31.0],
    [4, 51.0, 22.0],
    [5, 73.0, 35.0],
    [6, 89.0, 42.0]

f"{data: chart(bar), x(month), y(revenue, profit)}"
"#;
    let result = compile_and_execute(source).expect("should compile and run");
    let spec = chart_result(
        &result,
        "chart-formatted Table<T> row literals should return chart content",
    );
    assert_eq!(spec.chart_type, ChartType::Bar);
    assert_eq!(spec.x_label.as_deref(), Some("month"));
    assert_channel(spec, "x", "month", &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    assert_channel(spec, "y", "revenue", &[42.0, 58.0, 65.0, 51.0, 73.0, 89.0]);
    assert_channel(spec, "y", "profit", &[18.0, 25.0, 31.0, 22.0, 35.0, 42.0]);
}

#[test]
fn test_table_row_literal_single_row() {
    // MED-8: Single-row table literal should create a table, not an array
    let source = r#"
type Record { id: int, value: int, name: string }
let t: Table<Record> = [1, 100, "alpha"]
t.count()
"#;
    let result = compile_and_execute(source).expect("should compile and run");
    assert_eq!(
        result.as_i64().or(result.as_f64().map(|f| f as i64)),
        Some(1),
        "Single-row table literal should create a table with 1 row"
    );
}

#[test]
fn test_table_row_literal_single_row_filter() {
    // Single-row table should support methods like filter
    let source = r#"
type SalesRow { month: int, revenue: int }
let t: Table<SalesRow> = [1, 42]
let filtered = t.filter(|row| row.revenue > 30)
filtered.count()
"#;
    let result = compile_and_execute(source).expect("should compile and run");
    assert_eq!(
        result.as_i64().or(result.as_f64().map(|f| f as i64)),
        Some(1)
    );
}

// ===== MED-6: select(lambda) on DataTable =====

#[test]
fn test_table_select_with_lambda() {
    // Direct row-field projections lower statically to DataTable column select.
    let source = r#"
type Record { id: int, value: int, name: string }
let t: Table<Record> = [1, 100, "alpha"], [2, 200, "beta"]
let projected = t.select(|row| { id: row.id })
projected.count()
"#;
    let result = compile_and_execute(source).expect("should compile and run");
    assert_eq!(
        result.as_i64().or(result.as_f64().map(|f| f as i64)),
        Some(2),
        "select(lambda) should produce a table with same row count"
    );
}

#[test]
fn test_table_select_with_string_still_works() {
    // Ensure string-based select still works after adding lambda support
    let source = r#"
type Record { id: int, value: int, name: string }
let t: Table<Record> = [1, 100, "alpha"], [2, 200, "beta"]
let projected = t.select("id", "name")
projected.count()
"#;
    let result = compile_and_execute(source).expect("should compile and run");
    assert_eq!(
        result.as_i64().or(result.as_f64().map(|f| f as i64)),
        Some(2),
        "select(string) should preserve the table row count"
    );
}

// ===== MED-7: Improved error message for select returning non-object =====

#[test]
fn test_table_select_lambda_scalar_builds_value_column() {
    // A scalar direct field projection lowers statically to a one-column select.
    let source = r#"
type Record { id: int, value: int, name: string }
let t: Table<Record> = [1, 100, "alpha"], [2, 200, "beta"]
let projected = t.select(|row| row.id)
projected.count()
"#;
    let result = compile_and_execute(source).expect("should compile and run");
    assert_eq!(
        result.as_i64().or(result.as_f64().map(|f| f as i64)),
        Some(2)
    );
}

// --- MED-25: .clone() method on arrays ---

#[test]
fn test_array_clone_method() {
    // arr.clone() should produce a shallow copy identical to the original
    let source = r#"
        let arr = [1, 2, 3]
        let cloned = arr.clone()
        cloned.len()
    "#;
    let result = compile_and_execute(source).unwrap();
    assert_eq!(
        result.as_i64(),
        Some(3),
        "cloned array should have length 3"
    );
}

#[test]
fn test_array_clone_method_preserves_elements() {
    let source = r#"
        let arr = [10, 20, 30]
        let cloned = arr.clone()
        cloned.sum()
    "#;
    let result = compile_and_execute(source).unwrap();
    // sum of [10, 20, 30] = 60
    let val = result
        .as_i64()
        .or_else(|| result.as_f64().map(|f| f as i64));
    assert_eq!(val, Some(60), "cloned array sum should be 60");
}

// --- LOW-4: extend block to_string() should shadow builtin ---

#[test]
fn test_extend_to_string_shadows_builtin() {
    // A user-defined to_string in an extend block should take precedence
    // over the builtin formatting path.
    let source = r#"
        type Greeting { name: string }

        extend Greeting {
            method to_string() -> string {
                f"Hello, {self.name}!"
            }
        }

        let g = Greeting { name: "World" }
        g.to_string()
    "#;
    let result = compile_and_execute(source).unwrap();
    let s = result.as_str().expect("should return string");
    assert_eq!(s, "Hello, World!", "extend to_string should shadow builtin");
}
