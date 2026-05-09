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
