//! Integration tests for the `csv` stdlib module via Shape source code.
//!
//! NOTE: The csv module is defined as a ModuleExports but is NOT yet registered
//! as a VM extension (unlike crypto, json, set, msgpack). These tests verify
//! the module functions work correctly by using the `use std::core::csv` import path
//! which routes through the module loader.
//!
//! Currently csv is not registered in the VM, so these tests use the direct
//! Rust API. If csv gets registered as a VM extension in the future, these
//! can be converted to Shape source-level tests.

use shape_runtime::stdlib::csv_module::create_csv_module;
use shape_value::{ValueWord, ValueWordExt};
use std::sync::Arc;

fn test_ctx() -> shape_runtime::module_exports::ModuleContext<'static> {
    let registry = Box::leak(Box::new(
        shape_runtime::type_schema::TypeSchemaRegistry::new(),
    ));
    shape_runtime::module_exports::ModuleContext {
        schemas: registry,
        invoke_callable: None,
        raw_invoker: None,
        function_hashes: None,
        vm_state: None,
        granted_permissions: None,
        scope_constraints: None,
        set_pending_resume: None,
        set_pending_frame_resume: None,
    }
}

#[test]
fn csv_parse_basic() {
    let module = create_csv_module();
    let parse_fn = module.get_export("parse").unwrap();
    let ctx = test_ctx();
    let input = ValueWord::from_string(Arc::new("name,age\nAlice,30\nBob,25".to_string()));
    let result = parse_fn(&[input], &ctx).unwrap();
    let rows = result.as_any_array().expect("should be array").to_generic();
    assert_eq!(rows.len(), 3);
}

#[test]
fn csv_parse_field_access() {
    let module = create_csv_module();
    let parse_fn = module.get_export("parse").unwrap();
    let ctx = test_ctx();
    let input = ValueWord::from_string(Arc::new("a,b,c\n1,2,3".to_string()));
    let result = parse_fn(&[input], &ctx).unwrap();
    let rows = result.as_any_array().expect("should be array").to_generic();
    let header = rows[0]
        .as_any_array()
        .expect("row should be array")
        .to_generic();
    assert_eq!(header[0].as_str(), Some("a"));
    assert_eq!(header[1].as_str(), Some("b"));
    assert_eq!(header[2].as_str(), Some("c"));
}

#[test]
fn csv_parse_empty() {
    let module = create_csv_module();
    let parse_fn = module.get_export("parse").unwrap();
    let ctx = test_ctx();
    let input = ValueWord::from_string(Arc::new("".to_string()));
    let result = parse_fn(&[input], &ctx).unwrap();
    let rows = result.as_any_array().expect("should be array").to_generic();
    assert_eq!(rows.len(), 0);
}

#[test]
fn csv_parse_records_basic() {
    let module = create_csv_module();
    let parse_fn = module.get_export("parse_records").unwrap();
    let ctx = test_ctx();
    let input = ValueWord::from_string(Arc::new("name,age\nAlice,30\nBob,25".to_string()));
    let result = parse_fn(&[input], &ctx).unwrap();
    let records = result.as_any_array().expect("should be array").to_generic();
    assert_eq!(records.len(), 2);
}

#[test]
fn csv_stringify_roundtrip() {
    let module = create_csv_module();
    let parse_fn = module.get_export("parse").unwrap();
    let stringify_fn = module.get_export("stringify").unwrap();
    let ctx = test_ctx();

    let original = "x,y\n1,2\n";
    let parsed = parse_fn(
        &[ValueWord::from_string(Arc::new(original.to_string()))],
        &ctx,
    )
    .unwrap();
    let back = stringify_fn(&[parsed], &ctx).unwrap();
    assert_eq!(back.as_str(), Some(original));
}

#[test]
fn csv_is_valid_true() {
    let module = create_csv_module();
    let is_valid_fn = module.get_export("is_valid").unwrap();
    let ctx = test_ctx();
    let input = ValueWord::from_string(Arc::new("a,b,c\n1,2,3".to_string()));
    let result = is_valid_fn(&[input], &ctx).unwrap();
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn csv_is_valid_empty() {
    let module = create_csv_module();
    let is_valid_fn = module.get_export("is_valid").unwrap();
    let ctx = test_ctx();
    let input = ValueWord::from_string(Arc::new("".to_string()));
    let result = is_valid_fn(&[input], &ctx).unwrap();
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn csv_stringify_records_roundtrip() {
    let module = create_csv_module();
    let parse_records_fn = module.get_export("parse_records").unwrap();
    let stringify_records_fn = module.get_export("stringify_records").unwrap();
    let ctx = test_ctx();

    let csv_text = "name,age\nAlice,30\nBob,25\n";
    let parsed = parse_records_fn(
        &[ValueWord::from_string(Arc::new(csv_text.to_string()))],
        &ctx,
    )
    .unwrap();

    let headers = ValueWord::from_array(Arc::new(vec![
        ValueWord::from_string(Arc::new("name".to_string())),
        ValueWord::from_string(Arc::new("age".to_string())),
    ]));
    let back = stringify_records_fn(&[parsed, headers], &ctx).unwrap();
    assert_eq!(back.as_str(), Some(csv_text));
}

#[test]
fn csv_stringify_basic() {
    let module = create_csv_module();
    let stringify_fn = module.get_export("stringify").unwrap();
    let ctx = test_ctx();
    let data = ValueWord::from_array(Arc::new(vec![
        ValueWord::from_array(Arc::new(vec![
            ValueWord::from_string(Arc::new("a".to_string())),
            ValueWord::from_string(Arc::new("b".to_string())),
        ])),
        ValueWord::from_array(Arc::new(vec![
            ValueWord::from_string(Arc::new("1".to_string())),
            ValueWord::from_string(Arc::new("2".to_string())),
        ])),
    ]));
    let result = stringify_fn(&[data], &ctx).unwrap();
    assert_eq!(result.as_str(), Some("a,b\n1,2\n"));
}
