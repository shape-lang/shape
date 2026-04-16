//! Integration tests for the native `time` module.
//!
//! The `time` module is a Rust-level native module (not compiled from Shape
//! source), so these tests exercise the module export API directly.

use shape_runtime::module_exports::ModuleContext;
use shape_runtime::stdlib_time::create_time_module;
use shape_runtime::type_schema::TypeSchemaRegistry;
use shape_value::{ValueWord, ValueWordExt};
use std::sync::LazyLock;

/// Shared schema registry for all tests in this module.
static REGISTRY: LazyLock<TypeSchemaRegistry> = LazyLock::new(TypeSchemaRegistry::new);

fn test_ctx() -> ModuleContext<'static> {
    ModuleContext {
        schemas: &REGISTRY,
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
fn time_module_exports_all_functions() {
    let module = create_time_module();
    assert!(module.has_export("now"), "should export now");
    assert!(module.has_export("sleep"), "should export sleep");
    assert!(module.has_export("sleep_sync"), "should export sleep_sync");
    assert!(module.has_export("benchmark"), "should export benchmark");
    assert!(module.has_export("stopwatch"), "should export stopwatch");
    assert!(module.has_export("millis"), "should export millis");
}

#[test]
fn time_now_returns_instant() {
    let module = create_time_module();
    let now_fn = module.get_export("now").unwrap();
    let result = now_fn(&[], &test_ctx()).unwrap();
    assert_eq!(result.type_name(), "instant");
    assert!(result.as_instant().is_some());
}

#[test]
fn time_stopwatch_returns_instant() {
    let module = create_time_module();
    let sw_fn = module.get_export("stopwatch").unwrap();
    let result = sw_fn(&[], &test_ctx()).unwrap();
    assert_eq!(result.type_name(), "instant");
    assert!(result.as_instant().is_some());
}

#[test]
fn time_instant_elapsed_increases() {
    let module = create_time_module();
    let now_fn = module.get_export("now").unwrap();
    let instant = now_fn(&[], &test_ctx()).unwrap();

    std::thread::sleep(std::time::Duration::from_millis(5));

    let inst = instant.as_instant().unwrap();
    let elapsed = inst.elapsed();
    assert!(
        elapsed.as_nanos() > 0,
        "elapsed time should be > 0 after sleep"
    );
}

#[test]
fn time_instant_duration_since() {
    let module = create_time_module();
    let now_fn = module.get_export("now").unwrap();
    let ctx = test_ctx();

    let first = now_fn(&[], &ctx).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    let second = now_fn(&[], &ctx).unwrap();

    let first_inst = *first.as_instant().unwrap();
    let second_inst = *second.as_instant().unwrap();
    let duration = second_inst.duration_since(first_inst);
    assert!(
        duration.as_nanos() > 0,
        "duration_since should be > 0 for later instant"
    );
}

#[test]
fn time_millis_returns_epoch() {
    let module = create_time_module();
    let millis_fn = module.get_export("millis").unwrap();
    let result = millis_fn(&[], &test_ctx()).unwrap();
    let ms = result.as_f64().unwrap();
    // Should be after 2020-01-01 in milliseconds
    assert!(
        ms > 1_577_836_800_000.0,
        "epoch millis should be after 2020"
    );
}

#[test]
fn time_sleep_sync_zero_succeeds() {
    let module = create_time_module();
    let sleep_fn = module.get_export("sleep_sync").unwrap();
    let result = sleep_fn(&[ValueWord::from_f64(0.0)], &test_ctx());
    assert!(result.is_ok(), "sleep_sync(0) should succeed");
}

#[test]
fn time_sleep_sync_negative_errors() {
    let module = create_time_module();
    let sleep_fn = module.get_export("sleep_sync").unwrap();
    let result = sleep_fn(&[ValueWord::from_f64(-1.0)], &test_ctx());
    assert!(result.is_err(), "sleep_sync(-1) should error");
}

#[test]
fn time_sleep_sync_no_args_errors() {
    let module = create_time_module();
    let sleep_fn = module.get_export("sleep_sync").unwrap();
    let result = sleep_fn(&[], &test_ctx());
    assert!(result.is_err(), "sleep_sync() with no args should error");
}
