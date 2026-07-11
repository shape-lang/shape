//! Builtin schema definitions for fixed-layout runtime objects.
//!
//! These schemas replace the lazy runtime registration done by
//! `create_typed_object_from_pairs`. Each schema is registered once
//! at init with real field types and constant field indices.

use super::SchemaId;
use super::enum_support::EnumVariantInfo;
use super::field_types::FieldType;
use super::registry::{TypeSchemaBuilder, TypeSchemaRegistry};
use crate::comptime_reflection::FrozenTypeCategory;

/// Unspellable schema identity for compiler-issued comptime `TypeRef` values.
/// The SOH prefix cannot occur in a Shape identifier, so source code cannot
/// construct a lookalike nominal carrier.
pub const COMPTIME_FROZEN_TYPE_REF_SCHEMA: &str = "\u{1}comptime:TypeRef";

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

// -- Option (2 fields) --
pub const OPTION_VARIANT: usize = 0;
pub const OPTION_PAYLOAD: usize = 1;
pub const OPTION_VARIANT_SOME: i64 = 0;
pub const OPTION_VARIANT_NONE: i64 = 1;

// -- Result (2 fields) --
pub const RESULT_VARIANT: usize = 0;
pub const RESULT_PAYLOAD: usize = 1;
pub const RESULT_VARIANT_OK: i64 = 0;
pub const RESULT_VARIANT_ERR: i64 = 1;

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
    pub option: SchemaId,
    pub result: SchemaId,
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
        option: registry.get("__Option")?.id,
        result: registry.get("__Result")?.id,
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

    let option = TypeSchemaBuilder::new("__Option")
        .i64_field("variant")
        .any_field("payload")
        .register(registry);

    let result = TypeSchemaBuilder::new("__Result")
        .i64_field("variant")
        .any_field("payload")
        .register(registry);

    let empty_object = TypeSchemaBuilder::new("__EmptyObject").register(registry);

    // -- Comptime introspection contract schemas (comptime-excellence
    //    §4.1.4 / §4.3, S2 root cause B) --------------------------------
    //
    // Reserved, concrete, named schemas backing the comptime introspection
    // contract. Registered HERE — at registry init, before any user or
    // module (`__mod_*`) schema — so every registry that can host comptime
    // execution (the compiler's ambient registry AND each comptime/handler
    // VM's bytecode registry) assigns them the SAME low, deterministic id.
    // Descriptor construction resolves them BY NAME
    // (`typed_object_for_named_schema`); the baked-in `schema_id` is
    // therefore an id that means the same thing in the registry that
    // dereferences it. This removes the cross-registry schema-id reuse that
    // let `target.fields` / `field.name` resolve to an unrelated `__mod_*`
    // module-object schema at the same numeric id (audit's
    // `{is_valid, parse, stringify}` corruption).
    //
    // Concrete FieldTypes only — never `FieldType::Any` (R2 precedent: an
    // all-Any schema poisons static field-tag sourcing). Array/heap
    // carriers use `Array`/`String` informationally; the parallel
    // field-kind track + heap_mask drive the actual read. The `reserved`
    // flag makes ad-hoc field-set / field-order inference
    // (`lookup_schema_for_fields`) skip these, so an unrelated `{name,
    // kind, …}` object can never silently bind to a contract schema.
    // `comptime_api` (value 1) is the frozen introspection-contract version
    // marker (comptime-excellence §4.1.4): user annotation libraries feature-
    // gate against future contract revisions via `build_config().comptime_api`
    // without string-parsing `version`. Additive-only; appended last so the
    // existing field offsets (debug=0 … target_arch=3) stay stable.
    let _comptime_build_config = TypeSchemaBuilder::new("__ComptimeBuildConfig")
        .bool_field("debug")
        .string_field("version")
        .string_field("target_os")
        .string_field("target_arch")
        .int_field("comptime_api")
        .register(registry);

    let _comptime_type_ref = TypeSchemaBuilder::new("__ComptimeTypeRef")
        .string_field("name")
        .string_field("kind")
        .string_field("source")
        .register(registry);

    let _frozen_type_category = registry.register_enum_scoped(
        "FrozenTypeCategory",
        FrozenTypeCategory::ALL
            .into_iter()
            .enumerate()
            .map(|(id, category)| EnumVariantInfo::new(category.variant_name(), id as u16, 0))
            .collect(),
    );

    let _comptime_frozen_type_ref = TypeSchemaBuilder::new(COMPTIME_FROZEN_TYPE_REF_SCHEMA)
        .int_field("identity_high")
        .int_field("identity_low")
        .register(registry);

    let _comptime_item_fragment = TypeSchemaBuilder::new("__ComptimeItemFragment")
        .string_field("kind")
        .string_field("name")
        .string_field("return_type")
        .object_field("return_type_ref", "__ComptimeTypeRef")
        .string_field("literal_kind")
        .string_field("literal_string")
        .int_field("literal_int")
        .f64_field("literal_number")
        .bool_field("literal_bool")
        .register(registry);

    let _comptime_field_descriptor = TypeSchemaBuilder::new("__ComptimeFieldDescriptor")
        .string_field("name")
        .string_field("type")
        .array_field("annotations", FieldType::Any)
        .bool_field("optional")
        .object_field("type_ref", "__ComptimeTypeRef")
        .register(registry);

    let _comptime_param_descriptor = TypeSchemaBuilder::new("__ComptimeParamDescriptor")
        .string_field("name")
        .string_field("type")
        .bool_field("const")
        .object_field("type_ref", "__ComptimeTypeRef")
        .register(registry);

    let _comptime_annotation_descriptor = TypeSchemaBuilder::new("__ComptimeAnnotationDescriptor")
        .string_field("name")
        .array_field("args", FieldType::Any)
        .register(registry);

    // comptime-excellence §4.3 line 284: `return_type: OptionString` — a
    // function target with no declared return type produces `None`, stored
    // as a `NativeKind::Null` slot; a declared return type produces
    // `Some(rendered)`, stored as a `String` slot. Declaring the field
    // `Option<string>` (`FIELD_TAG_OPTION`) routes the read through the
    // carrier-authoritative `field_kinds` track (ADR-006 §2.7.7 / §2.7.26),
    // so the `None` case reads back as the stamped `Null` discriminator
    // instead of reinterpreting an absent value as a `FieldType::String`
    // heap pointer. Declaring it plain `String` (the pre-fix shape) left the
    // `None` read with no statically-sourceable kind → the coarse-tag read
    // path surfaced an internal error whose message displaced the user's
    // `error()` text (the `@llm_tool` missing-return-type guard, §4.9.2).
    let _comptime_target = TypeSchemaBuilder::new("__ComptimeTarget")
        .string_field("kind")
        .string_field("name")
        .array_field("fields", FieldType::Any)
        .array_field("params", FieldType::Any)
        .option_string_field("return_type")
        .object_field("return_type_ref", "__ComptimeTypeRef")
        .array_field("annotations", FieldType::Any)
        .array_field("captures", FieldType::Any)
        .register(registry);

    // TypeInfo record returned by the `type_info(T)` comptime builtin.
    // `__`-prefixed to avoid clashing with the user-visible stdlib
    // `TypeInfo` type (`stdlib-src/core/types.shape`). `name` / `kind` NAMES
    // and ORDER lead so the object's physical layout aligns with the first two
    // typed fields; `fields` follows as the declared-field descriptor array.
    // Field access on the reflection result resolves by NAME through this
    // schema, so appending `fields` does not disturb `name` / `kind` reads.
    let _comptime_type_info = TypeSchemaBuilder::new("__ComptimeTypeInfo")
        .string_field("name")
        .string_field("kind")
        // `fields` reuses the same `__ComptimeFieldDescriptor` row as
        // `target.fields` (comptime-excellence §4.1.2): one row shape, one
        // builder, no split introspection story. TypedObject kinds populate it
        // from the type's declared fields; every other kind is an empty array.
        .array_field("fields", FieldType::Any)
        .object_field("type_ref", "__ComptimeTypeRef")
        .register(registry);

    // §4.4 comptime-handler `ctx` compile-context record. Read-only build
    // context handed to `@comptime` blocks/handlers. Field NAMES + ORDER match
    // the handler's `ctx` param type annotation
    // (`annotation_comptime_ctx_type_annotation`) so typed field access
    // resolves the right offsets. (`ctx.build` is intentionally absent —
    // `build_config()` is the single build-info surface, §4.4.)
    let _comptime_context = TypeSchemaBuilder::new("__ComptimeContext")
        .string_field("module_path")
        .string_field("file")
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
        option,
        result,
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
        assert!(registry.has_type("__Option"));
        assert!(registry.has_type("__Result"));
        assert!(registry.has_type("__EmptyObject"));
        assert!(registry.has_type("__ComptimeBuildConfig"));
        assert!(registry.has_type("__ComptimeFieldDescriptor"));
        assert!(registry.has_type("__ComptimeParamDescriptor"));
        assert!(registry.has_type("__ComptimeAnnotationDescriptor"));
        assert!(registry.has_type("__ComptimeTarget"));
        assert!(registry.has_type("__ComptimeTypeInfo"));
        assert!(registry.has_type("__ComptimeTypeRef"));
        assert!(registry.has_type(COMPTIME_FROZEN_TYPE_REF_SCHEMA));
        assert!(registry.has_type("FrozenTypeCategory"));
        assert!(registry.has_type("__ComptimeItemFragment"));

        // Check field counts
        let any_error = registry.get_by_id(ids.any_error).unwrap();
        assert_eq!(any_error.field_count(), 6);

        let trace_frame = registry.get_by_id(ids.trace_frame).unwrap();
        assert_eq!(trace_frame.field_count(), 4);

        let option = registry.get_by_id(ids.option).unwrap();
        assert_eq!(option.field_count(), 2);

        let result = registry.get_by_id(ids.result).unwrap();
        assert_eq!(result.field_count(), 2);

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

        let schema = registry.get_by_id(ids.option).unwrap();
        assert_eq!(schema.fields[OPTION_VARIANT].name, "variant");
        assert_eq!(schema.fields[OPTION_PAYLOAD].name, "payload");

        let schema = registry.get_by_id(ids.result).unwrap();
        assert_eq!(schema.fields[RESULT_VARIANT].name, "variant");
        assert_eq!(schema.fields[RESULT_PAYLOAD].name, "payload");
    }

    #[test]
    fn test_option_result_carrier_tags_match_stdlib_enums() {
        let registry = TypeSchemaRegistry::with_stdlib_types();

        let option = registry.get("Option").unwrap();
        assert_eq!(
            option.variant_id("Some").map(i64::from),
            Some(OPTION_VARIANT_SOME)
        );
        assert_eq!(
            option.variant_id("None").map(i64::from),
            Some(OPTION_VARIANT_NONE)
        );

        let result = registry.get("Result").unwrap();
        assert_eq!(
            result.variant_id("Ok").map(i64::from),
            Some(RESULT_VARIANT_OK)
        );
        assert_eq!(
            result.variant_id("Err").map(i64::from),
            Some(RESULT_VARIANT_ERR)
        );
    }

    #[test]
    fn test_resolve_builtin_schema_ids_requires_registered_option_result() {
        let registry = TypeSchemaRegistry::new();
        assert!(resolve_builtin_schema_ids(&registry).is_none());

        let mut registry = TypeSchemaRegistry::new();
        let ids = register_builtin_schemas(&mut registry);
        let resolved = resolve_builtin_schema_ids(&registry).unwrap();
        assert_eq!(resolved.option, ids.option);
        assert_eq!(resolved.result, ids.result);
    }
}

#[cfg(test)]
#[path = "builtin_schemas/comptime_reflection_tests.rs"]
mod comptime_reflection_tests;
