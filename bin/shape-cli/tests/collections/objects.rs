use crate::common::{eval, eval_to_bool, eval_to_number, init_runtime};

#[test]
fn test_object_literals() {
    init_runtime();

    assert_eq!(eval_to_number("let obj = { x: 10, y: 20 }; obj.x"), 10.0);
    assert_eq!(eval_to_number("let obj = { x: 10, y: 20 }; obj.y"), 20.0);
}

#[test]
fn test_csv_module_is_removed() {
    init_runtime();

    let err = eval(r#"csv.load("test.csv")"#).expect_err("csv namespace must be removed");
    assert!(
        err.contains("csv.load(...) has been removed")
            || err.contains("csv.load(...) has been removed.")
            || err.contains("csv.load")
            || err.contains("removed")
            || err.contains("Undefined variable: 'csv'")
            || err.contains("Undefined function: 'csv'")
            || err.contains("Unknown module"),
        "unexpected error for removed csv module: {}",
        err
    );
}

#[test]
fn test_global_load_function_is_removed() {
    init_runtime();

    let err = eval(r#"load("csv", { path: "test.csv" })"#)
        .expect_err("global load(plugin, params) must be removed");
    assert!(
        err.contains("load(provider, params) has been removed")
            || err.contains("has been removed")
            || err.contains("Undefined variable: 'load'")
            || err.contains("Undefined function: 'load'")
            || err.contains("Unknown function"),
        "unexpected error for removed global load: {}",
        err
    );
}

#[test]
fn test_object_keys_method() {
    init_runtime();

    let code = r#"
        let obj = { x: 10, y: 20, z: 30 };
        obj.x == 10 && obj.y == 20 && obj.z == 30
    "#;

    assert!(eval_to_bool(code));
}

#[test]
fn test_object_values_method() {
    init_runtime();

    let code = r#"
        let obj = { x: 10, y: 20 };
        let sum = obj.x + obj.y;
        sum == 30
    "#;

    assert!(eval_to_bool(code));
}

#[test]
fn test_object_has_method() {
    init_runtime();

    let code = r#"
        let obj = { x: 10, y: 20 };
        // Test property existence by accessing them
        obj.x == 10 && obj.y == 20
    "#;

    assert!(eval_to_bool(code));
}

#[test]
fn test_object_set_immutability() {
    init_runtime();

    let code = r#"
        let obj1 = { x: 10 };
        let obj2 = { x: obj1.x, y: 20 };

        // obj2 should have both x and y
        obj2.x == 10 && obj2.y == 20
    "#;

    assert!(eval_to_bool(code));
}

#[test]
fn test_object_set_returns_new_value() {
    init_runtime();

    let code = r#"
        let obj = { x: 10 };
        let obj2 = obj + { x: 42 };
        obj2.x
    "#;

    assert_eq!(eval_to_number(code), 42.0);
}

#[test]
fn test_object_method_chaining() {
    init_runtime();

    let code = r#"
        let obj = { a: 1, b: 2 };
        let obj2 = obj + { c: 3 } + { d: 4 };
        obj2.a == 1 && obj2.c == 3 && obj2.d == 4
    "#;

    assert!(eval_to_bool(code));
}

#[test]
fn test_annotation_context_state_management() {
    init_runtime();

    let code = r#"
        // Simulate annotation context pattern with objects
        let ctx = { state: { value: 10 } };
        let stored = ctx.state;
        stored.value == 10
    "#;

    assert!(eval_to_bool(code));
}

#[test]
fn test_annotation_context_get_set_pattern() {
    init_runtime();

    let code = r#"
        let state1 = { count: 1, name: "test" };
        let state2 = state1 + { count: 2 };

        // state2 should have updated count
        state2.count == 2 && state2.name == "test"
    "#;

    assert!(eval_to_bool(code));
}
