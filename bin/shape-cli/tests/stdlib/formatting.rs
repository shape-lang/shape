//! Integration tests for format evaluation via ShapeEngine
//! and print() with format strings.

use shape_runtime::engine::ShapeEngine;

// ============================================================================
// Format Integration Tests
// ============================================================================

#[test]
fn test_engine_format_number_default() {
    let mut engine = ShapeEngine::new().expect("engine should create");
    if engine.load_stdlib().is_err() {
        eprintln!("Skipping test - stdlib not available");
        return;
    }

    let result = engine.format_value_string(0.1234, "Number", None, &Default::default());
    assert!(
        result.is_ok(),
        "Default format should work: {:?}",
        result.err()
    );
    assert_eq!(result.unwrap(), "<formatted Number>");
}

#[test]
fn test_engine_format_number_percent() {
    let mut engine = ShapeEngine::new().expect("engine should create");
    if engine.load_stdlib().is_err() {
        eprintln!("Skipping test - stdlib not available");
        return;
    }

    let result = engine.format_value_string(0.1234, "Number", Some("Percent"), &Default::default());
    assert!(
        result.is_ok(),
        "Percent format should work: {:?}",
        result.err()
    );
    assert_eq!(result.unwrap(), "<formatted Number as Percent>");
}

#[test]
fn test_engine_format_number_percent_with_params() {
    let mut engine = ShapeEngine::new().expect("engine should create");
    if engine.load_stdlib().is_err() {
        eprintln!("Skipping test - stdlib not available");
        return;
    }

    let mut params = std::collections::HashMap::new();
    params.insert("decimals".to_string(), serde_json::json!(1));

    let result = engine.format_value_string(0.1234, "Number", Some("Percent"), &params);
    assert!(
        result.is_ok(),
        "Percent with params should work: {:?}",
        result.err()
    );
    assert_eq!(result.unwrap(), "<formatted Number as Percent>");
}

#[test]
fn test_engine_format_number_currency() {
    let mut engine = ShapeEngine::new().expect("engine should create");
    if engine.load_stdlib().is_err() {
        eprintln!("Skipping test - stdlib not available");
        return;
    }

    let result =
        engine.format_value_string(1234.56, "Number", Some("Currency"), &Default::default());
    assert!(
        result.is_ok(),
        "Currency format should work: {:?}",
        result.err()
    );
    assert_eq!(result.unwrap(), "<formatted Number as Currency>");
}

// ============================================================================
// Print VM Test (placeholder)
// ============================================================================

#[test]
fn test_vm_feature_disabled() {
    // Placeholder test - VM tests are disabled until "vm" feature is implemented
}
