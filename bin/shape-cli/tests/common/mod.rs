//! Common test helpers for shape-cli integration tests.
//!
//! Extracted from shape-core/tests/feature_coverage.rs.
#![allow(dead_code)]

use shape_runtime::initialize_shared_runtime;
use shape_runtime::engine::ShapeEngine;
use shape_vm::BytecodeExecutor;

pub fn init_runtime() {
    let _ = initialize_shared_runtime();
}

pub fn eval(code: &str) -> Result<serde_json::Value, String> {
    let mut engine = ShapeEngine::new().map_err(|e| e.to_string())?;
    engine.load_stdlib().map_err(|e| e.to_string())?;
    let mut executor = BytecodeExecutor::new();
    let result = engine.execute(&mut executor, code).map_err(|e| e.to_string())?;
    serde_json::to_value(&result.value).map_err(|e| e.to_string())
}

fn value_to_number(value: serde_json::Value) -> f64 {
    match value {
        serde_json::Value::Number(n) => n.as_f64().unwrap(),
        serde_json::Value::Object(map) if map.contains_key("Integer") => match &map["Integer"] {
            serde_json::Value::Number(n) => n.as_f64().unwrap(),
            other => panic!("Expected number in Object, got: {:?}", other),
        },
        serde_json::Value::Object(map) if map.contains_key("Number") => match &map["Number"] {
            serde_json::Value::Number(n) => n.as_f64().unwrap(),
            other => panic!("Expected number in Object, got: {:?}", other),
        },
        other => panic!("Expected number, got: {:?}", other),
    }
}

fn value_to_bool(value: serde_json::Value) -> bool {
    match value {
        serde_json::Value::Bool(b) => b,
        serde_json::Value::Object(map) if map.contains_key("Bool") => match &map["Bool"] {
            serde_json::Value::Bool(b) => *b,
            other => panic!("Expected bool in Object, got: {:?}", other),
        },
        other => panic!("Expected bool, got: {:?}", other),
    }
}

fn value_to_string(value: serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s,
        serde_json::Value::Object(map) if map.contains_key("String") => match &map["String"] {
            serde_json::Value::String(s) => s.clone(),
            other => panic!("Expected string in Object, got: {:?}", other),
        },
        other => panic!("Expected string, got: {:?}", other),
    }
}

pub fn eval_to_number(code: &str) -> f64 {
    value_to_number(eval(code).unwrap_or_else(|e| panic!("Expected number, got error: {}", e)))
}

pub fn eval_to_bool(code: &str) -> bool {
    value_to_bool(eval(code).unwrap_or_else(|e| panic!("Expected bool, got error: {}", e)))
}

pub fn eval_to_string(code: &str) -> String {
    value_to_string(eval(code).unwrap_or_else(|e| panic!("Expected string, got error: {}", e)))
}
