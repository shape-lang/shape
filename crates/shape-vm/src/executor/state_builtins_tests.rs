// Tests for the `std::state` module builtins.
//
// **Phase-2c rebuild pending — see ADR-006 §2.7.4.** The pre-bulldozer
// suite called every `state_*` body directly with hand-built
// `ValueWord` arguments. After the kinded migration, those bodies
// panic via `todo!()` until the snapshot/diff rebuild lands; calling
// them from tests would only assert that `todo!()` panics, which is
// not an interesting signal.
//
// The kinded-shape rewrite of the suite — driving each body through
// `register_typed_function`'s wrapper with `&[KindedSlot]` inputs and
// `TypedReturn` outputs, plus a kind-threaded `serializable_to_slot`
// inverse for the round-trip cases — is part of the same Phase-2c
// rebuild scope. Until that lands, every body-level test is replaced
// with a single `#[ignore]`'d placeholder so the test surface is
// preserved without exercising deleted APIs.
//
// The schema-construction surface (`create_state_module` registers
// the expected schemas, every export name is present) is exercisable
// independent of body migration; the live tests below cover that
// surface.

use super::*;

#[test]
fn test_create_state_module_exports() {
    let module = create_state_module();
    assert_eq!(module.name, "std::core::state");
    assert!(module.has_export("hash"));
    assert!(module.has_export("fn_hash"));
    assert!(module.has_export("schema_hash"));
    assert!(module.has_export("serialize"));
    assert!(module.has_export("deserialize"));
    assert!(module.has_export("diff"));
    assert!(module.has_export("patch"));
    assert!(module.has_export("capture"));
    assert!(module.has_export("capture_all"));
    assert!(module.has_export("capture_module"));
    assert!(module.has_export("capture_call"));
    assert!(module.has_export("resume"));
    assert!(module.has_export("resume_frame"));
    assert!(module.has_export("caller"));
    assert!(module.has_export("args"));
    assert!(module.has_export("locals"));
    assert!(module.has_export("snapshot"));
}

// ---------------------------------------------------------------------------
// Phase-2c body-level coverage placeholders
// ---------------------------------------------------------------------------
//
// Every test below was a body-level assertion in the pre-bulldozer
// suite. The post-Phase-2c rebuild needs to:
//   1. Drive each body through `register_typed_function`'s
//      `&[KindedSlot]` -> `TypedReturn` wrapper rather than calling
//      the body function directly.
//   2. Replace `ValueWord::from_*` constructors at the call site with
//      `KindedSlot::from_*` plus the matching `NativeKind`.
//   3. Round-trip cases (serialize/deserialize, diff/patch) need the
//      kind-threaded slot-serialization helpers that §2.7.4 defers to
//      Phase 2c.
//
// Until that rebuild, the tests are `#[ignore]`'d to preserve the
// list of intended assertions without exercising the deleted APIs.

#[test]
#[ignore = "phase-2c — state-snapshot rebuild — see ADR-006 §2.7.4"]
fn test_state_hash_deterministic() {}

#[test]
#[ignore = "phase-2c — state-snapshot rebuild — see ADR-006 §2.7.4"]
fn test_state_hash_different_values() {}

#[test]
#[ignore = "phase-2c — state-snapshot rebuild — see ADR-006 §2.7.4"]
fn test_state_hash_returns_hex_string() {}

#[test]
#[ignore = "phase-2c — state-snapshot rebuild — see ADR-006 §2.7.4"]
fn test_state_fn_hash_with_function() {}

#[test]
#[ignore = "phase-2c — state-snapshot rebuild — see ADR-006 §2.7.4"]
fn test_state_fn_hash_non_function() {}

#[test]
#[ignore = "phase-2c — state-snapshot rebuild — see ADR-006 §2.7.4"]
fn test_state_serialize_deserialize_roundtrip_number() {}

#[test]
#[ignore = "phase-2c — state-snapshot rebuild — see ADR-006 §2.7.4"]
fn test_state_serialize_deserialize_roundtrip_string() {}

#[test]
#[ignore = "phase-2c — state-snapshot rebuild — see ADR-006 §2.7.4"]
fn test_state_serialize_deserialize_roundtrip_bool() {}

#[test]
#[ignore = "phase-2c — state-snapshot rebuild — see ADR-006 §2.7.4"]
fn test_state_serialize_deserialize_roundtrip_array() {}

#[test]
#[ignore = "phase-2c — state-snapshot rebuild — see ADR-006 §2.7.4"]
fn test_state_serialize_deserialize_none() {}

#[test]
#[ignore = "phase-2c — state-snapshot rebuild — see ADR-006 §2.7.4"]
fn test_state_diff_identical() {}

#[test]
#[ignore = "phase-2c — state-snapshot rebuild — see ADR-006 §2.7.4"]
fn test_state_diff_changed() {}

#[test]
#[ignore = "phase-2c — state-snapshot rebuild — see ADR-006 §2.7.4"]
fn test_state_patch_root_replacement_legacy_array() {}

#[test]
#[ignore = "phase-2c — state-snapshot rebuild — see ADR-006 §2.7.4"]
fn test_state_diff_patch_roundtrip() {}

/// W17-snapshot-resume gate test: every `state.*` body returns a
/// structured `Err(...)` carrying the W17 surface text, never a
/// `todo!()` panic that would abort the VM thread.
///
/// The pre-W17 bodies were `todo!()` macros; this test would have
/// aborted the test process. Post-W17 they return `Err(String)` with
/// a structured surface message — this test exercises every entry
/// point in `state_builtins/introspection.rs` and the content/serialize/
/// diff family in `state_builtins/core.rs` and asserts that:
///   (a) the call returns `Err(_)` rather than panicking, and
///   (b) the error message carries the W17 surface marker so audit
///       trails can locate the deferral.
#[test]
fn test_w17_state_bodies_return_structured_errors() {
    use crate::executor::state_builtins::core::{
        state_deserialize, state_diff, state_fn_hash, state_hash, state_patch,
        state_schema_hash, state_serialize,
    };
    use crate::executor::state_builtins::introspection::{
        state_args_stub, state_caller_stub, state_capture_all_stub, state_capture_call_stub,
        state_capture_module_stub, state_capture_stub, state_locals_stub,
        state_resume_frame_stub, state_resume_stub,
    };
    use shape_runtime::module_exports::ModuleContext;
    use shape_runtime::type_schema::TypeSchemaRegistry;

    let schemas = TypeSchemaRegistry::default();
    let ctx = ModuleContext {
        schemas: &schemas,
        invoke_callable: None,
        raw_invoker: None,
        function_hashes: None,
        vm_state: None,
        granted_permissions: None,
        scope_constraints: None,
        set_pending_resume: None,
        set_pending_frame_resume: None,
    };

    // Every state body returns Err(String) with the W17 marker. We
    // pass an empty slot slice — none of these bodies actually inspect
    // their args, they surface-stop immediately.
    let empty_args: &[shape_value::KindedSlot] = &[];

    let fixtures: &[(
        &str,
        fn(
            &[shape_value::KindedSlot],
            &ModuleContext,
        ) -> Result<
            shape_runtime::typed_module_exports::TypedReturn,
            String,
        >,
    )] = &[
        ("state.capture", state_capture_stub),
        ("state.capture_all", state_capture_all_stub),
        ("state.capture_module", state_capture_module_stub),
        ("state.capture_call", state_capture_call_stub),
        ("state.resume", state_resume_stub),
        ("state.resume_frame", state_resume_frame_stub),
        ("state.caller", state_caller_stub),
        ("state.args", state_args_stub),
        ("state.locals", state_locals_stub),
        ("state.hash", state_hash),
        ("state.fn_hash", state_fn_hash),
        ("state.schema_hash", state_schema_hash),
        ("state.serialize", state_serialize),
        ("state.deserialize", state_deserialize),
        ("state.diff", state_diff),
        ("state.patch", state_patch),
    ];

    for (name, body) in fixtures {
        let result = body(empty_args, &ctx);
        let err = result.as_ref().err().unwrap_or_else(|| {
            panic!(
                "{name}: expected Err(...) surface, got Ok(...) — W17 \
                 surface-and-stop expects every state.* body to return \
                 a structured error until Phase-2c rebuild lands"
            )
        });
        assert!(
            err.contains("W17-snapshot-resume surface"),
            "{name}: error message missing W17 surface marker; got: {err}"
        );
        assert!(
            err.contains("§2.7.4"),
            "{name}: error message missing ADR-006 §2.7.4 cite; got: {err}"
        );
    }
}

#[test]
#[ignore = "phase-2c — state-snapshot rebuild — see ADR-006 §2.7.4"]
fn test_capture_stubs_return_errors() {}

#[test]
#[ignore = "phase-2c — state-snapshot rebuild — see ADR-006 §2.7.4"]
fn test_state_args_returns_captured_args() {}

#[test]
#[ignore = "phase-2c — state-snapshot rebuild — see ADR-006 §2.7.4"]
fn test_state_locals_returns_name_value_pairs() {}

#[test]
#[ignore = "phase-2c — state-snapshot rebuild — see ADR-006 §2.7.4"]
fn test_state_caller_returns_caller_frame() {}

#[test]
#[ignore = "phase-2c — state-snapshot rebuild — see ADR-006 §2.7.4"]
fn test_state_caller_returns_none_when_no_caller() {}
