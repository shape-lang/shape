use super::*;
use crate::executor::{VMConfig, VirtualMachine};
use arrow_schema::{DataType, Field, Schema};
use shape_value::ValueWord;
use shape_value::datatable::DataTableBuilder;
use std::collections::HashMap;
use std::sync::Arc;

fn make_vm() -> VirtualMachine {
    VirtualMachine::new(VMConfig::default())
}

/// Slot-based TypedObject to HashMap conversion for test assertions.
/// Looks up schemas from: program registry, then module_binding anon cache.
fn to_obj_map(
    val: &ValueWord,
    vm: &VirtualMachine,
) -> std::collections::HashMap<String, ValueWord> {
    if let Some((schema_id, slots, heap_mask)) = val.as_typed_object() {
        let sid = schema_id as u32;
        let schema = vm.lookup_schema(sid);
        if let Some(schema) = schema {
            let mut map = HashMap::with_capacity(schema.fields.len());
            for (i, field_def) in schema.fields.iter().enumerate() {
                if i < slots.len() {
                    let val = crate::executor::objects::object_creation::read_slot_value_typed(
                        slots,
                        i,
                        heap_mask,
                        Some(&field_def.field_type),
                    );
                    map.insert(field_def.name.clone(), val);
                }
            }
            return map;
        }
    }
    // Fall back to typed_object_to_hashmap (for anon-schema objects)
    shape_runtime::type_schema::typed_object_to_hashmap(val).expect("Expected object-like value")
}

fn to_nb_args(args: Vec<ValueWord>) -> Vec<ValueWord> {
    args.into_iter().map(|v| v).collect()
}

fn predeclared_object(fields: &[(&str, ValueWord)]) -> ValueWord {
    let field_names: Vec<String> = fields.iter().map(|(name, _)| (*name).to_string()).collect();
    let _ = shape_runtime::type_schema::register_predeclared_any_schema(&field_names);
    shape_runtime::type_schema::typed_object_from_pairs(fields)
}

fn sample_dt() -> Arc<shape_value::datatable::DataTable> {
    let schema = Schema::new(vec![
        Field::new("price", DataType::Float64, false),
        Field::new("volume", DataType::Int64, false),
        Field::new("symbol", DataType::Utf8, false),
    ]);
    let mut builder = DataTableBuilder::new(schema);
    builder
        .add_f64_column(vec![100.0, 200.0, 150.0, 50.0])
        .add_i64_column(vec![1000, 2000, 1500, 500])
        .add_string_column(vec!["AAPL", "GOOG", "AAPL", "GOOG"]);
    Arc::new(builder.finish().unwrap())
}

// =========================================================================
// filter() tests
// =========================================================================

#[test]
fn test_filter_f64_gt() {
    let mut vm = make_vm();
    let dt = sample_dt();
    let args = vec![
        ValueWord::from_datatable(dt),
        ValueWord::from_string(Arc::new("price".to_string())),
        ValueWord::from_string(Arc::new(">".to_string())),
        ValueWord::from_f64(100.0),
    ];
    let result = handle_filter(&mut vm, to_nb_args(args), None).unwrap();
    let dt = result.as_datatable().expect("Expected DataTable");
    assert_eq!(dt.row_count(), 2);
    let prices = dt.get_f64_column("price").unwrap();
    assert_eq!(prices.value(0), 200.0);
    assert_eq!(prices.value(1), 150.0);
}

#[test]
fn test_filter_f64_lt() {
    let mut vm = make_vm();
    let dt = sample_dt();
    let args = vec![
        ValueWord::from_datatable(dt),
        ValueWord::from_string(Arc::new("price".to_string())),
        ValueWord::from_string(Arc::new("<".to_string())),
        ValueWord::from_f64(150.0),
    ];
    let result = handle_filter(&mut vm, to_nb_args(args), None).unwrap();
    let dt = result.as_datatable().expect("Expected DataTable");
    assert_eq!(dt.row_count(), 2);
    let prices = dt.get_f64_column("price").unwrap();
    assert_eq!(prices.value(0), 100.0);
    assert_eq!(prices.value(1), 50.0);
}

#[test]
fn test_filter_f64_eq() {
    let mut vm = make_vm();
    let dt = sample_dt();
    let args = vec![
        ValueWord::from_datatable(dt),
        ValueWord::from_string(Arc::new("price".to_string())),
        ValueWord::from_string(Arc::new("==".to_string())),
        ValueWord::from_f64(100.0),
    ];
    let result = handle_filter(&mut vm, to_nb_args(args), None).unwrap();
    let dt = result.as_datatable().expect("Expected DataTable");
    assert_eq!(dt.row_count(), 1);
    assert_eq!(dt.get_f64_column("price").unwrap().value(0), 100.0);
}

#[test]
fn test_filter_string_eq() {
    let mut vm = make_vm();
    let dt = sample_dt();
    let args = vec![
        ValueWord::from_datatable(dt),
        ValueWord::from_string(Arc::new("symbol".to_string())),
        ValueWord::from_string(Arc::new("==".to_string())),
        ValueWord::from_string(Arc::new("AAPL".to_string())),
    ];
    let result = handle_filter(&mut vm, to_nb_args(args), None).unwrap();
    let dt = result.as_datatable().expect("Expected DataTable");
    assert_eq!(dt.row_count(), 2);
    let symbols = dt.get_string_column("symbol").unwrap();
    assert_eq!(symbols.value(0), "AAPL");
    assert_eq!(symbols.value(1), "AAPL");
}

#[test]
fn test_filter_no_matches() {
    let mut vm = make_vm();
    let dt = sample_dt();
    let args = vec![
        ValueWord::from_datatable(dt),
        ValueWord::from_string(Arc::new("price".to_string())),
        ValueWord::from_string(Arc::new(">".to_string())),
        ValueWord::from_f64(9999.0),
    ];
    let result = handle_filter(&mut vm, to_nb_args(args), None).unwrap();
    let dt = result.as_datatable().expect("Expected DataTable");
    assert_eq!(dt.row_count(), 0);
}

#[test]
fn test_filter_empty_table() {
    let mut vm = make_vm();
    let schema = Schema::new(vec![Field::new("x", DataType::Float64, false)]);
    let mut builder = DataTableBuilder::new(schema);
    builder.add_f64_column(vec![]);
    let dt = Arc::new(builder.finish().unwrap());
    let args = vec![
        ValueWord::from_datatable(dt),
        ValueWord::from_string(Arc::new("x".to_string())),
        ValueWord::from_string(Arc::new(">".to_string())),
        ValueWord::from_f64(0.0),
    ];
    let result = handle_filter(&mut vm, to_nb_args(args), None).unwrap();
    let dt = result.as_datatable().expect("Expected DataTable");
    assert_eq!(dt.row_count(), 0);
}

#[test]
fn test_filter_i64_gte() {
    let mut vm = make_vm();
    let dt = sample_dt();
    let args = vec![
        ValueWord::from_datatable(dt),
        ValueWord::from_string(Arc::new("volume".to_string())),
        ValueWord::from_string(Arc::new(">=".to_string())),
        ValueWord::from_i64(1500),
    ];
    let result = handle_filter(&mut vm, to_nb_args(args), None).unwrap();
    let dt = result.as_datatable().expect("Expected DataTable");
    assert_eq!(dt.row_count(), 2);
    let vols = dt.get_i64_column("volume").unwrap();
    assert_eq!(vols.value(0), 2000);
    assert_eq!(vols.value(1), 1500);
}

// =========================================================================
// orderBy() tests
// =========================================================================

#[test]
fn test_order_by_ascending() {
    let mut vm = make_vm();
    let dt = sample_dt();
    let args = vec![
        ValueWord::from_datatable(dt),
        ValueWord::from_string(Arc::new("price".to_string())),
    ];
    let result = handle_order_by(&mut vm, to_nb_args(args), None).unwrap();
    let dt = result.as_datatable().expect("Expected DataTable");
    let prices = dt.get_f64_column("price").unwrap();
    assert_eq!(prices.value(0), 50.0);
    assert_eq!(prices.value(1), 100.0);
    assert_eq!(prices.value(2), 150.0);
    assert_eq!(prices.value(3), 200.0);
}

#[test]
fn test_order_by_descending() {
    let mut vm = make_vm();
    let dt = sample_dt();
    let args = vec![
        ValueWord::from_datatable(dt),
        ValueWord::from_string(Arc::new("price".to_string())),
        ValueWord::from_string(Arc::new("desc".to_string())),
    ];
    let result = handle_order_by(&mut vm, to_nb_args(args), None).unwrap();
    let dt = result.as_datatable().expect("Expected DataTable");
    let prices = dt.get_f64_column("price").unwrap();
    assert_eq!(prices.value(0), 200.0);
    assert_eq!(prices.value(1), 150.0);
    assert_eq!(prices.value(2), 100.0);
    assert_eq!(prices.value(3), 50.0);
}

#[test]
fn test_order_by_string() {
    let mut vm = make_vm();
    let dt = sample_dt();
    let args = vec![
        ValueWord::from_datatable(dt),
        ValueWord::from_string(Arc::new("symbol".to_string())),
    ];
    let result = handle_order_by(&mut vm, to_nb_args(args), None).unwrap();
    let dt = result.as_datatable().expect("Expected DataTable");
    let symbols = dt.get_string_column("symbol").unwrap();
    assert_eq!(symbols.value(0), "AAPL");
    assert_eq!(symbols.value(1), "AAPL");
    assert_eq!(symbols.value(2), "GOOG");
    assert_eq!(symbols.value(3), "GOOG");
}

// =========================================================================
// group_by() tests
// =========================================================================

#[test]
fn test_group_by_string() {
    let mut vm = make_vm();
    let dt = sample_dt();
    let args = vec![
        ValueWord::from_datatable(dt),
        ValueWord::from_string(Arc::new("symbol".to_string())),
    ];
    let result = handle_group_by(&mut vm, to_nb_args(args), None).unwrap();
    let groups = result.to_generic_array().expect("Expected Array");
    assert_eq!(groups.len(), 2);
    // Groups sorted by key: AAPL, GOOG
    let g0 = to_obj_map(&groups[0].clone(), &vm);
    assert_eq!(
        g0["key"],
        ValueWord::from_string(Arc::new("AAPL".to_string()))
    );
    assert_eq!(
        g0["group"]
            .as_datatable()
            .expect("Expected DataTable in group")
            .row_count(),
        2
    );
    let g1 = to_obj_map(&groups[1].clone(), &vm);
    assert_eq!(
        g1["key"],
        ValueWord::from_string(Arc::new("GOOG".to_string()))
    );
    assert_eq!(
        g1["group"]
            .as_datatable()
            .expect("Expected DataTable in group")
            .row_count(),
        2
    );
}

#[test]
fn test_group_by_empty() {
    let mut vm = make_vm();
    let schema = Schema::new(vec![Field::new("x", DataType::Utf8, false)]);
    let mut builder = DataTableBuilder::new(schema);
    builder.add_string_column(vec![]);
    let dt = Arc::new(builder.finish().unwrap());
    let args = vec![
        ValueWord::from_datatable(dt),
        ValueWord::from_string(Arc::new("x".to_string())),
    ];
    let result = handle_group_by(&mut vm, to_nb_args(args), None).unwrap();
    let groups = result.to_generic_array().expect("Expected Array");
    assert_eq!(groups.len(), 0);
}

// =========================================================================
// aggregate() tests
// =========================================================================

#[test]
fn test_aggregate_sum_mean() {
    let mut vm = make_vm();
    let dt = sample_dt();
    let spec = predeclared_object(&[
        (
            "total_price",
            ValueWord::from_array(Arc::new(vec![
                ValueWord::from_string(Arc::new("sum".to_string())),
                ValueWord::from_string(Arc::new("price".to_string())),
            ])),
        ),
        (
            "avg_price",
            ValueWord::from_array(Arc::new(vec![
                ValueWord::from_string(Arc::new("mean".to_string())),
                ValueWord::from_string(Arc::new("price".to_string())),
            ])),
        ),
    ]);
    let args = vec![ValueWord::from_datatable(dt), spec];
    let result = handle_aggregate(&mut vm, to_nb_args(args), None).unwrap();
    let dt = result.as_datatable().expect("Expected DataTable");
    assert_eq!(dt.row_count(), 1);
    let avg = dt.get_f64_column("avg_price").unwrap().value(0);
    assert!((avg - 125.0).abs() < 0.001);
    let total = dt.get_f64_column("total_price").unwrap().value(0);
    assert!((total - 500.0).abs() < 0.001);
}

#[test]
fn test_aggregate_min_max_count() {
    let mut vm = make_vm();
    let dt = sample_dt();
    let spec = predeclared_object(&[
        (
            "min_price",
            ValueWord::from_array(Arc::new(vec![
                ValueWord::from_string(Arc::new("min".to_string())),
                ValueWord::from_string(Arc::new("price".to_string())),
            ])),
        ),
        (
            "max_price",
            ValueWord::from_array(Arc::new(vec![
                ValueWord::from_string(Arc::new("max".to_string())),
                ValueWord::from_string(Arc::new("price".to_string())),
            ])),
        ),
        (
            "n",
            ValueWord::from_array(Arc::new(vec![
                ValueWord::from_string(Arc::new("count".to_string())),
                ValueWord::from_string(Arc::new("price".to_string())),
            ])),
        ),
    ]);
    let args = vec![ValueWord::from_datatable(dt), spec];
    let result = handle_aggregate(&mut vm, to_nb_args(args), None).unwrap();
    let dt = result.as_datatable().expect("Expected DataTable");
    assert_eq!(dt.row_count(), 1);
    let min_p = dt.get_f64_column("min_price").unwrap().value(0);
    assert!((min_p - 50.0).abs() < 0.001);
    let max_p = dt.get_f64_column("max_price").unwrap().value(0);
    assert!((max_p - 200.0).abs() < 0.001);
    let n = dt.get_i64_column("n").unwrap().value(0);
    assert_eq!(n, 4);
}

#[test]
fn test_aggregate_shorthand() {
    let mut vm = make_vm();
    let dt = sample_dt();
    let spec =
        predeclared_object(&[("price", ValueWord::from_string(Arc::new("sum".to_string())))]);
    let args = vec![ValueWord::from_datatable(dt), spec];
    let result = handle_aggregate(&mut vm, to_nb_args(args), None).unwrap();
    let dt = result.as_datatable().expect("Expected DataTable");
    assert_eq!(dt.row_count(), 1);
    let total = dt.get_f64_column("price").unwrap().value(0);
    assert!((total - 500.0).abs() < 0.001);
}

// =========================================================================
// count() tests
// =========================================================================

#[test]
fn test_count() {
    let mut vm = make_vm();
    let dt = sample_dt();
    let args = vec![ValueWord::from_datatable(dt)];
    let result = handle_count(&mut vm, to_nb_args(args), None).unwrap();
    assert_eq!(result, ValueWord::from_i64(4));
}

#[test]
fn test_count_empty() {
    let mut vm = make_vm();
    let schema = Schema::new(vec![Field::new("x", DataType::Float64, false)]);
    let mut builder = DataTableBuilder::new(schema);
    builder.add_f64_column(vec![]);
    let dt = Arc::new(builder.finish().unwrap());
    let args = vec![ValueWord::from_datatable(dt)];
    let result = handle_count(&mut vm, to_nb_args(args), None).unwrap();
    assert_eq!(result, ValueWord::from_i64(0));
}

#[test]
fn test_to_mat_default_numeric_columns() {
    let mut vm = make_vm();
    let dt = sample_dt();
    let args = vec![ValueWord::from_datatable(dt)];
    let result = handle_to_mat(&mut vm, to_nb_args(args), None).unwrap();
    let mat = result.as_matrix().expect("Expected Matrix");
    assert_eq!(mat.rows, 4);
    assert_eq!(mat.cols, 2);

    // Row 0: [100.0, 1000.0]
    assert_eq!(mat.data[0], 100.0);
    assert_eq!(mat.data[1], 1000.0);

    // Row 3: [50.0, 500.0]
    assert_eq!(mat.data[6], 50.0);
    assert_eq!(mat.data[7], 500.0);
}

#[test]
fn test_to_mat_selected_column() {
    let mut vm = make_vm();
    let dt = sample_dt();
    let args = vec![
        ValueWord::from_datatable(dt),
        ValueWord::from_string(Arc::new("volume".to_string())),
    ];
    let result = handle_to_mat(&mut vm, to_nb_args(args), None).unwrap();
    let mat = result.as_matrix().expect("Expected Matrix");
    assert_eq!(mat.rows, 4);
    assert_eq!(mat.cols, 1);
    // Row 1, Col 0 => 2000.0
    assert_eq!(mat.data[1], 2000.0);
}

#[test]
fn test_to_mat_rejects_non_numeric_column() {
    let mut vm = make_vm();
    let dt = sample_dt();
    let args = vec![
        ValueWord::from_datatable(dt),
        ValueWord::from_string(Arc::new("symbol".to_string())),
    ];
    assert!(handle_to_mat(&mut vm, to_nb_args(args), None).is_err());
}

// =========================================================================
// describe() tests
// =========================================================================

#[test]
fn test_describe() {
    let mut vm = make_vm();
    let dt = sample_dt();
    let args = vec![ValueWord::from_datatable(dt)];
    let result = handle_describe(&mut vm, to_nb_args(args), None).unwrap();
    let dt = result.as_datatable().expect("Expected DataTable");
    // 5 stat rows: count, mean, min, max, sum
    assert_eq!(dt.row_count(), 5);
    // Columns: stat, price, volume
    assert_eq!(dt.column_count(), 3);
    let stats = dt.get_string_column("stat").unwrap();
    assert_eq!(stats.value(0), "count");
    assert_eq!(stats.value(1), "mean");
    assert_eq!(stats.value(2), "min");
    assert_eq!(stats.value(3), "max");
    assert_eq!(stats.value(4), "sum");

    let price_stats = dt.get_f64_column("price").unwrap();
    assert!((price_stats.value(0) - 4.0).abs() < 0.001); // count
    assert!((price_stats.value(1) - 125.0).abs() < 0.001); // mean
    assert!((price_stats.value(2) - 50.0).abs() < 0.001); // min
    assert!((price_stats.value(3) - 200.0).abs() < 0.001); // max
    assert!((price_stats.value(4) - 500.0).abs() < 0.001); // sum
}

// =========================================================================
// forEach() — basic test (no closure execution, just validates the call pattern)
// =========================================================================

#[test]
fn test_for_each_requires_function() {
    let mut vm = make_vm();
    let dt = sample_dt();
    // Missing function argument
    let args = vec![ValueWord::from_datatable(dt)];
    assert!(handle_for_each(&mut vm, to_nb_args(args), None).is_err());
}

#[test]
fn test_for_each_rejects_non_function() {
    let mut vm = make_vm();
    let dt = sample_dt();
    let args = vec![ValueWord::from_datatable(dt), ValueWord::from_f64(42.0)];
    assert!(handle_for_each(&mut vm, to_nb_args(args), None).is_err());
}

// =========================================================================
// Helper tests
// =========================================================================

#[test]
fn test_apply_comparison_neq() {
    let mut vm = make_vm();
    let dt = sample_dt();
    let args = vec![
        ValueWord::from_datatable(dt),
        ValueWord::from_string(Arc::new("symbol".to_string())),
        ValueWord::from_string(Arc::new("!=".to_string())),
        ValueWord::from_string(Arc::new("AAPL".to_string())),
    ];
    let result = handle_filter(&mut vm, to_nb_args(args), None).unwrap();
    let dt = result.as_datatable().expect("Expected DataTable");
    assert_eq!(dt.row_count(), 2);
    let symbols = dt.get_string_column("symbol").unwrap();
    assert_eq!(symbols.value(0), "GOOG");
    assert_eq!(symbols.value(1), "GOOG");
}

#[test]
fn test_filter_invalid_operator() {
    let mut vm = make_vm();
    let dt = sample_dt();
    let args = vec![
        ValueWord::from_datatable(dt),
        ValueWord::from_string(Arc::new("price".to_string())),
        ValueWord::from_string(Arc::new("~=".to_string())),
        ValueWord::from_f64(100.0),
    ];
    assert!(handle_filter(&mut vm, to_nb_args(args), None).is_err());
}

// =========================================================================
// simulate() tests
// =========================================================================

#[test]
fn test_simulate_event_log_and_seed() {
    use crate::bytecode::{BytecodeProgram, Constant, Function, Instruction, OpCode, Operand};

    // Build a handler function: (row, state, idx) => { state: state, result: "signal", event_type: "trade" }
    // The function body starts at instruction index 1 (after the main program's Halt)
    let handler_entry = 1;
    let mut program = BytecodeProgram::default();
    let handler_schema_id = program.type_schema_registry.register_type(
        "__test_sim_handler",
        vec![
            (
                "state".to_string(),
                shape_runtime::type_schema::FieldType::Any,
            ),
            (
                "result".to_string(),
                shape_runtime::type_schema::FieldType::Any,
            ),
            (
                "event_type".to_string(),
                shape_runtime::type_schema::FieldType::Any,
            ),
        ],
    );
    let handler_schema_u16 =
        u16::try_from(handler_schema_id).expect("test schema id should fit in u16");

    let instructions = vec![
        // Main program: Halt at index 0 (never reached in this test)
        Instruction::simple(OpCode::Halt),
        // Handler body starts at index 1:
        // Push field values for object: { state: <local1>, result: "signal", event_type: "trade" }
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(1))), // state arg
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // "signal"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // "trade"
        Instruction::new(
            OpCode::NewTypedObject,
            Some(Operand::TypedObjectAlloc {
                schema_id: handler_schema_u16,
                field_count: 3,
            }),
        ),
        Instruction::simple(OpCode::ReturnValue),
    ];
    let constants = vec![
        Constant::String("signal".to_string()),
        Constant::String("trade".to_string()),
    ];
    let functions = vec![Function {
        name: "handler".to_string(),
        arity: 3,
        param_names: vec!["row".to_string(), "state".to_string(), "idx".to_string()],
        locals_count: 3,
        entry_point: handler_entry,
        body_length: 0,
        is_closure: false,
        captures_count: 0,
        is_async: false,
        ref_params: Vec::new(),
        ref_mutates: Vec::new(),
        mutable_captures: Vec::new(),
        frame_descriptor: None,
        osr_entry_points: Vec::new(),
        mir_data: None,
    }];

    program.instructions = instructions;
    program.constants = constants;
    program.functions = functions;

    let mut vm = VirtualMachine::new(crate::executor::VMConfig::default());
    vm.load_program(program);

    let dt = sample_dt();

    // Build config: { initial_state: 0, collect_event_log: true, seed: 42 }
    let config = predeclared_object(&[
        ("initial_state", ValueWord::from_i64(0)),
        ("collect_event_log", ValueWord::from_bool(true)),
        ("seed", ValueWord::from_i64(42)),
    ]);

    let args = vec![
        ValueWord::from_datatable(dt),
        ValueWord::from_function(0), // handler function id
        config,
    ];

    let result = handle_simulate(&mut vm, to_nb_args(args), None).unwrap();

    let obj = to_obj_map(&result, &vm);
    // Verify completed
    assert_eq!(obj["completed"], ValueWord::from_bool(true));
    // Verify elements_processed
    assert_eq!(obj["elements_processed"], ValueWord::from_i64(4));
    // Verify seed passthrough
    assert_eq!(obj["seed"], ValueWord::from_i64(42));

    // Verify event_log exists and has 4 entries (one per row)
    let events = obj["event_log"]
        .to_generic_array()
        .expect("Expected Array for event_log");
    assert_eq!(events.len(), 4);
    // Check first event
    let e0 = to_obj_map(&events[0].clone(), &vm);
    assert_eq!(e0["idx"], ValueWord::from_i64(0));
    assert_eq!(
        e0["event_type"],
        ValueWord::from_string(Arc::new("trade".to_string()))
    );
    assert_eq!(
        e0["result"],
        ValueWord::from_string(Arc::new("signal".to_string()))
    );
    // Check last event has idx = 3
    let e3 = to_obj_map(&events[3].clone(), &vm);
    assert_eq!(e3["idx"], ValueWord::from_i64(3));

    // Verify results also collected
    let results = obj["results"]
        .to_generic_array()
        .expect("Expected Array for results");
    assert_eq!(results.len(), 4);
    assert_eq!(
        results[0].clone(),
        ValueWord::from_string(Arc::new("signal".to_string()))
    );
}

#[test]
fn test_simulate_requires_function() {
    let mut vm = make_vm();
    let dt = sample_dt();
    let args = vec![
        ValueWord::from_datatable(dt),
        ValueWord::from_string(Arc::new("not_a_function".to_string())),
    ];
    let result = handle_simulate(&mut vm, to_nb_args(args), None);
    assert!(result.is_err());
}

#[test]
fn test_simulate_returns_result_object() {
    // We can't easily test with a real closure without compiling Shape code,
    // but we can verify the function signature check works
    let mut vm = make_vm();
    let dt = sample_dt();
    let args = vec![ValueWord::from_datatable(dt)]; // No handler
    let result = handle_simulate(&mut vm, to_nb_args(args), None);
    assert!(result.is_err());
}

// =========================================================================
// rows() tests
// =========================================================================

#[test]
fn test_rows_returns_array_of_row_views() {
    let mut vm = make_vm();
    let dt = sample_dt();
    let args = vec![ValueWord::from_datatable(dt.clone())];
    let result = handle_rows(&mut vm, to_nb_args(args), None).unwrap();
    let arr = result.to_generic_array().expect("Expected array");
    assert_eq!(arr.len(), 4);
    // Each element should be a RowView
    for (i, row) in arr.iter().enumerate() {
        let (schema_id, table, row_idx) = row.as_row_view().expect("Expected RowView");
        assert_eq!(schema_id, 0);
        assert_eq!(row_idx, i);
        assert!(Arc::ptr_eq(table, &dt));
    }
}

#[test]
fn test_rows_empty_table() {
    let mut vm = make_vm();
    let schema = Schema::new(vec![Field::new("x", DataType::Float64, false)]);
    let mut builder = DataTableBuilder::new(schema);
    builder.add_f64_column(vec![]);
    let dt = Arc::new(builder.finish().unwrap());
    let args = vec![ValueWord::from_datatable(dt)];
    let result = handle_rows(&mut vm, to_nb_args(args), None).unwrap();
    let arr = result.to_generic_array().expect("Expected array");
    assert_eq!(arr.len(), 0);
}

#[test]
fn test_rows_typed_table_preserves_schema_id() {
    let mut vm = make_vm();
    let dt = sample_dt();
    let schema_id = 42u64;
    let args = vec![ValueWord::from_typed_table(schema_id, dt.clone())];
    let result = handle_rows(&mut vm, to_nb_args(args), None).unwrap();
    let arr = result.to_generic_array().expect("Expected array");
    assert_eq!(arr.len(), 4);
    for row in arr.iter() {
        let (sid, _, _) = row.as_row_view().expect("Expected RowView");
        assert_eq!(sid, schema_id);
    }
}

// =========================================================================
// columnsRef() tests
// =========================================================================

#[test]
fn test_columns_ref_returns_array_of_column_refs() {
    let mut vm = make_vm();
    let dt = sample_dt();
    let args = vec![ValueWord::from_datatable(dt.clone())];
    let result = handle_columns_ref(&mut vm, to_nb_args(args), None).unwrap();
    let arr = result.to_generic_array().expect("Expected array");
    assert_eq!(arr.len(), 3); // price, volume, symbol
    for (i, col) in arr.iter().enumerate() {
        let (schema_id, table, col_id) = col.as_column_ref().expect("Expected ColumnRef");
        assert_eq!(schema_id, 0);
        assert_eq!(col_id, i as u32);
        assert!(Arc::ptr_eq(table, &dt));
    }
}

#[test]
fn test_columns_ref_empty_table() {
    let mut vm = make_vm();
    let schema = Schema::new(vec![Field::new("x", DataType::Float64, false)]);
    let mut builder = DataTableBuilder::new(schema);
    builder.add_f64_column(vec![]);
    let dt = Arc::new(builder.finish().unwrap());
    let args = vec![ValueWord::from_datatable(dt)];
    let result = handle_columns_ref(&mut vm, to_nb_args(args), None).unwrap();
    let arr = result.to_generic_array().expect("Expected array");
    assert_eq!(arr.len(), 1); // "x" column still exists even with 0 rows
}

#[test]
fn test_columns_ref_typed_table_preserves_schema_id() {
    let mut vm = make_vm();
    let dt = sample_dt();
    let schema_id = 99u64;
    let args = vec![ValueWord::from_typed_table(schema_id, dt.clone())];
    let result = handle_columns_ref(&mut vm, to_nb_args(args), None).unwrap();
    let arr = result.to_generic_array().expect("Expected array");
    assert_eq!(arr.len(), 3);
    for col in arr.iter() {
        let (sid, _, _) = col.as_column_ref().expect("Expected ColumnRef");
        assert_eq!(sid, schema_id);
    }
}

// =========================================================================
// MED-6: select() with string columns
// =========================================================================

#[test]
fn test_select_string_columns() {
    let mut vm = make_vm();
    let dt = sample_dt();
    let args = vec![
        ValueWord::from_datatable(dt),
        ValueWord::from_string(Arc::new("price".to_string())),
        ValueWord::from_string(Arc::new("symbol".to_string())),
    ];
    let result = handle_select(&mut vm, to_nb_args(args), None).unwrap();
    let dt = result.as_datatable().expect("Expected DataTable");
    assert_eq!(dt.column_count(), 2);
    assert_eq!(dt.row_count(), 4);
}

#[test]
fn test_select_rejects_non_string_non_callable() {
    let mut vm = make_vm();
    let dt = sample_dt();
    // Passing a number (not a string and not a function)
    let args = vec![ValueWord::from_datatable(dt), ValueWord::from_f64(42.0)];
    let result = handle_select(&mut vm, to_nb_args(args), None);
    assert!(result.is_err());
    let err = format!("{:?}", result.unwrap_err());
    assert!(
        err.contains("select()"),
        "Error should mention select(): {}",
        err
    );
}

// =========================================================================
// MED-7: build_datatable_from_objects_nb scalar result
// =========================================================================

#[test]
fn test_build_datatable_from_scalar_rows() {
    // When build_datatable_from_objects_nb receives scalar rows,
    // it should build a single-column "value" table instead of erroring.
    let mut vm = make_vm();
    let rows = vec![ValueWord::from_i64(42), ValueWord::from_i64(99)];
    let result = common::build_datatable_from_objects_nb(&mut vm, &rows);
    assert!(result.is_ok(), "scalar rows should produce a table");
    let top = result.unwrap();
    let dt = top.as_datatable().expect("result should be a datatable");
    assert_eq!(dt.row_count(), 2);
    assert_eq!(dt.column_names(), vec!["value"]);
}
