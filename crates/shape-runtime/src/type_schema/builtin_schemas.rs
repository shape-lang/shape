//! Builtin schema definitions for fixed-layout runtime objects.
//!
//! These schemas replace the lazy runtime registration done by
//! `create_typed_object_from_pairs`. Each schema is registered once
//! at init with real field types and constant field indices.

use super::SchemaId;
use super::registry::{TypeSchemaBuilder, TypeSchemaRegistry};

// =========================================================================
// Field index constants
// =========================================================================

// -- AnyError (6 fields) --
pub const ANYERROR_CATEGORY: usize = 0;
pub const ANYERROR_PAYLOAD: usize = 1;
pub const ANYERROR_CAUSE: usize = 2;
pub const ANYERROR_TRACE_INFO: usize = 3;
pub const ANYERROR_MESSAGE: usize = 4;
pub const ANYERROR_CODE: usize = 5;

// -- TraceFrame (4 fields) --
pub const TRACEFRAME_IP: usize = 0;
pub const TRACEFRAME_LINE: usize = 1;
pub const TRACEFRAME_FILE: usize = 2;
pub const TRACEFRAME_FUNCTION: usize = 3;

// -- TraceInfoFull (2 fields) --
pub const TRACEINFO_FULL_KIND: usize = 0;
pub const TRACEINFO_FULL_FRAMES: usize = 1;

// -- TraceInfoSingle (2 fields) --
pub const TRACEINFO_SINGLE_KIND: usize = 0;
pub const TRACEINFO_SINGLE_FRAME: usize = 1;

// -- ReflectAnnotation (2 fields) --
pub const REFLECT_ANN_NAME: usize = 0;
pub const REFLECT_ANN_ARGS: usize = 1;

// -- ReflectField (3 fields) --
pub const REFLECT_FIELD_NAME: usize = 0;
pub const REFLECT_FIELD_TYPE: usize = 1;
pub const REFLECT_FIELD_ANNOTATIONS: usize = 2;

// -- ReflectResult (2 fields) --
pub const REFLECT_RESULT_NAME: usize = 0;
pub const REFLECT_RESULT_FIELDS: usize = 1;

// -- GroupResult (2 fields) --
pub const GROUP_RESULT_KEY: usize = 0;
pub const GROUP_RESULT_GROUP: usize = 1;

// -- EventLogEntry (3 fields) --
pub const EVENT_LOG_IDX: usize = 0;
pub const EVENT_LOG_EVENT_TYPE: usize = 1;
pub const EVENT_LOG_RESULT: usize = 2;

// -- SimulateReturn (6 fields) --
pub const SIM_RETURN_FINAL_STATE: usize = 0;
pub const SIM_RETURN_RESULTS: usize = 1;
pub const SIM_RETURN_ELEMENTS_PROCESSED: usize = 2;
pub const SIM_RETURN_COMPLETED: usize = 3;
pub const SIM_RETURN_EVENT_LOG: usize = 4;
pub const SIM_RETURN_SEED: usize = 5;

// =========================================================================
// BuiltinSchemaIds — one ID per fixed-layout schema
// =========================================================================

/// Schema IDs for all builtin fixed-layout schemas.
/// Populated at init, stored on VirtualMachine for fast access.
#[derive(Debug, Clone)]
pub struct BuiltinSchemaIds {
    pub any_error: SchemaId,
    pub trace_frame: SchemaId,
    pub trace_info_full: SchemaId,
    pub trace_info_single: SchemaId,
    pub reflect_annotation: SchemaId,
    pub reflect_field: SchemaId,
    pub reflect_result: SchemaId,
    pub group_result: SchemaId,
    pub event_log_entry: SchemaId,
    pub simulate_return: SchemaId,
    pub empty_object: SchemaId,
}

/// Resolve builtin schema IDs from an existing registry without registering new
/// schemas. Returns `None` when any required schema is missing.
pub fn resolve_builtin_schema_ids(registry: &TypeSchemaRegistry) -> Option<BuiltinSchemaIds> {
    Some(BuiltinSchemaIds {
        any_error: registry.get("__AnyError")?.id,
        trace_frame: registry.get("__TraceFrame")?.id,
        trace_info_full: registry.get("__TraceInfoFull")?.id,
        trace_info_single: registry.get("__TraceInfoSingle")?.id,
        reflect_annotation: registry.get("__ReflectAnnotation")?.id,
        reflect_field: registry.get("__ReflectField")?.id,
        reflect_result: registry.get("__ReflectResult")?.id,
        group_result: registry.get("__GroupResult")?.id,
        event_log_entry: registry.get("__EventLogEntry")?.id,
        simulate_return: registry.get("__SimulateReturn")?.id,
        empty_object: registry.get("__EmptyObject")?.id,
    })
}

// =========================================================================
// Registration
// =========================================================================

/// Register all builtin schemas into the given registry and return their IDs.
///
/// Field types: heap-allocated polymorphic fields use `FieldType::String`
/// (informational — the `heap_mask` bitmap determines actual read path).
pub fn register_builtin_schemas(registry: &mut TypeSchemaRegistry) -> BuiltinSchemaIds {
    let any_error = TypeSchemaBuilder::new("__AnyError")
        .string_field("category")
        .string_field("payload")
        .string_field("cause")
        .string_field("trace_info")
        .string_field("message")
        .string_field("code")
        .register(registry);

    let trace_frame = TypeSchemaBuilder::new("__TraceFrame")
        .string_field("ip")
        .string_field("line")
        .string_field("file")
        .string_field("function")
        .register(registry);

    let trace_info_full = TypeSchemaBuilder::new("__TraceInfoFull")
        .string_field("kind")
        .string_field("frames")
        .register(registry);

    let trace_info_single = TypeSchemaBuilder::new("__TraceInfoSingle")
        .string_field("kind")
        .string_field("frame")
        .register(registry);

    let reflect_annotation = TypeSchemaBuilder::new("__ReflectAnnotation")
        .string_field("name")
        .string_field("args")
        .register(registry);

    let reflect_field = TypeSchemaBuilder::new("__ReflectField")
        .string_field("name")
        .string_field("type")
        .string_field("annotations")
        .register(registry);

    let reflect_result = TypeSchemaBuilder::new("__ReflectResult")
        .string_field("name")
        .string_field("fields")
        .register(registry);

    let group_result = TypeSchemaBuilder::new("__GroupResult")
        .any_field("key")
        .any_field("group")
        .register(registry);

    let event_log_entry = TypeSchemaBuilder::new("__EventLogEntry")
        .i64_field("idx")
        .string_field("event_type")
        .any_field("result")
        .register(registry);

    let simulate_return = TypeSchemaBuilder::new("__SimulateReturn")
        .any_field("final_state")
        .any_field("results")
        .i64_field("elements_processed")
        .bool_field("completed")
        .any_field("event_log")
        .any_field("seed")
        .register(registry);

    let empty_object = TypeSchemaBuilder::new("__EmptyObject").register(registry);

    // Internal comptime helper object shapes
    let _comptime_build_config = TypeSchemaBuilder::new("__ComptimeBuildConfig")
        .bool_field("debug")
        .string_field("version")
        .string_field("target_os")
        .string_field("target_arch")
        .register(registry);

    let _comptime_target_field = TypeSchemaBuilder::new("__ComptimeTargetField")
        .string_field("name")
        .string_field("type")
        .register(registry);

    let _comptime_target_param = TypeSchemaBuilder::new("__ComptimeTargetParam")
        .string_field("name")
        .string_field("type")
        .bool_field("const")
        .register(registry);

    let _comptime_target = TypeSchemaBuilder::new("__ComptimeTarget")
        .string_field("kind")
        .string_field("name")
        .any_field("fields")
        .any_field("params")
        .any_field("return_type")
        .any_field("annotations")
        .any_field("captures")
        .register(registry);

    BuiltinSchemaIds {
        any_error,
        trace_frame,
        trace_info_full,
        trace_info_single,
        reflect_annotation,
        reflect_field,
        reflect_result,
        group_result,
        event_log_entry,
        simulate_return,
        empty_object,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_schemas_register() {
        let mut registry = TypeSchemaRegistry::new();
        let ids = register_builtin_schemas(&mut registry);

        // All schemas should be registered
        assert!(registry.has_type("__AnyError"));
        assert!(registry.has_type("__TraceFrame"));
        assert!(registry.has_type("__TraceInfoFull"));
        assert!(registry.has_type("__TraceInfoSingle"));
        assert!(registry.has_type("__ReflectAnnotation"));
        assert!(registry.has_type("__ReflectField"));
        assert!(registry.has_type("__ReflectResult"));
        assert!(registry.has_type("__GroupResult"));
        assert!(registry.has_type("__EventLogEntry"));
        assert!(registry.has_type("__SimulateReturn"));
        assert!(registry.has_type("__EmptyObject"));
        assert!(registry.has_type("__ComptimeBuildConfig"));
        assert!(registry.has_type("__ComptimeTargetField"));
        assert!(registry.has_type("__ComptimeTargetParam"));
        assert!(registry.has_type("__ComptimeTarget"));

        // Check field counts
        let any_error = registry.get_by_id(ids.any_error).unwrap();
        assert_eq!(any_error.field_count(), 6);

        let trace_frame = registry.get_by_id(ids.trace_frame).unwrap();
        assert_eq!(trace_frame.field_count(), 4);

        let empty = registry.get_by_id(ids.empty_object).unwrap();
        assert_eq!(empty.field_count(), 0);
    }

    #[test]
    fn test_field_indices_match_schema_order() {
        let mut registry = TypeSchemaRegistry::new();
        let ids = register_builtin_schemas(&mut registry);

        let schema = registry.get_by_id(ids.any_error).unwrap();
        assert_eq!(schema.fields[ANYERROR_CATEGORY].name, "category");
        assert_eq!(schema.fields[ANYERROR_PAYLOAD].name, "payload");
        assert_eq!(schema.fields[ANYERROR_CAUSE].name, "cause");
        assert_eq!(schema.fields[ANYERROR_TRACE_INFO].name, "trace_info");
        assert_eq!(schema.fields[ANYERROR_MESSAGE].name, "message");
        assert_eq!(schema.fields[ANYERROR_CODE].name, "code");

        let schema = registry.get_by_id(ids.simulate_return).unwrap();
        assert_eq!(schema.fields[SIM_RETURN_FINAL_STATE].name, "final_state");
        assert_eq!(schema.fields[SIM_RETURN_COMPLETED].name, "completed");
        assert_eq!(schema.fields[SIM_RETURN_SEED].name, "seed");
    }
}
