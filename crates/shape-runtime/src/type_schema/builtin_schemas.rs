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

mod generated_capture;
pub use generated_capture::COMPTIME_CAPTURE_DESCRIPTOR_SCHEMA;

/// Unspellable schema identity for compiler-issued comptime `TypeRef` values.
/// The SOH prefix cannot occur in a Shape identifier, so source code cannot
/// construct a lookalike nominal carrier.
pub const COMPTIME_FROZEN_TYPE_REF_SCHEMA: &str = "\u{1}comptime:TypeRef";

/// ADR-009 B1 S1 — unspellable schema identity for the payload-bearing
/// `FrozenType` sealed indexed sum returned by `reflect(TypeRef<T>)`
/// (Dec 50/94). Declares ONLY the enabled payload variants
/// (`FROZEN_TYPE_ENABLED_PAYLOAD_CATEGORIES`), with variant ids pinned to
/// the 10-category catalog ordinals.
pub const COMPTIME_FROZEN_TYPE_SCHEMA: &str = "\u{1}comptime:FrozenType";

/// ADR-009 B1 S1 — unspellable schema identity for the sealed
/// `FrozenPrimitive` sub-algebra (the `FrozenType::Primitive` payload).
/// Generated from the shared runtime catalog
/// (`comptime_reflection::FROZEN_PRIMITIVE_VARIANTS`).
pub const COMPTIME_FROZEN_PRIMITIVE_SCHEMA: &str = "\u{1}comptime:FrozenPrimitive";

/// ADR-009 B1 S1 — unspellable schema identity for the `FrozenNever` marker
/// descriptor (the `FrozenType::Never` payload). Zero fields.
pub const COMPTIME_FROZEN_NEVER_SCHEMA: &str = "\u{1}comptime:FrozenNever";

/// ADR-009 B1 S1 — unspellable schema identity for the `FrozenErased`
/// descriptor (the `FrozenType::Erased` payload). Carries the bound-set
/// array only — reachable today solely as the empty set via `any` (A2
/// unlanded); no field pretends `dyn Trait` bounds exist (spec §3.7).
pub const COMPTIME_FROZEN_ERASED_SCHEMA: &str = "\u{1}comptime:FrozenErased";

/// Unspellable schema identity for compiler-issued comptime `TraitRef`
/// values (ADR-009 ticket B2, Dec 49). A trait is not a value type: the
/// TraitRef carrier is a DISTINCT identity kind from the TypeRef carrier,
/// with its own reserved schema.
pub const COMPTIME_FROZEN_TRAIT_REF_SCHEMA: &str = "\u{1}comptime:TraitRef";

/// Unspellable schema identity for compiler-issued comptime `ImplRef`
/// implementation evidence (ADR-009 ticket B2, Dec 49). Carries the frozen
/// identity halves of the exact `(trait, type)` pair PLUS the impl's own
/// canonical identity, so evidence is tied to the exact pair and the exact
/// (possibly named) impl whose canonical identity enters generated-artifact
/// descriptor fingerprints.
pub const COMPTIME_FROZEN_IMPL_REF_SCHEMA: &str = "\u{1}comptime:ImplRef";

/// ADR-009 B4 (Stage 2, Dec 54) — unspellable schema identity for a compiler-
/// issued `TypeConstructorRef` (`type_constructor(C)`). Carries the frozen
/// nominal HEAD identity halves only: the ordered parameter kinds are a freeze
/// fact re-read through the single param-kind projection at `.apply(...)` time,
/// never a second table snapshotted into the carrier (CLAUDE.md "no second
/// arity/param-kind table"). The SOH prefix keeps it unspellable so source can
/// never forge a constructor.
pub const COMPTIME_FROZEN_TYPE_CONSTRUCTOR_REF_SCHEMA: &str = "\u{1}comptime:TypeConstructorRef";

/// ADR-009 B4 (Stage 2, Dec 54) — unspellable schema identity for an
/// `AppliedType` (the result of `constructor.apply(args)`). Carries the applied
/// identity halves (identity-EQUAL to the A2 `type_ref(Head<Args>)` spelling),
/// the frozen head identity halves, and the ordered argument identities as an
/// interleaved `high, low, …` int array — enough to `refine(...)` the
/// application and read `type_argument(i)` WITHOUT inverting the one-way
/// SHA-256 identity hash. Identities only: no descriptor strings, no partial
/// descriptors (spec §3.1).
pub const COMPTIME_APPLIED_TYPE_SCHEMA: &str = "\u{1}comptime:AppliedType";

/// ADR-009 B6 (Stage 2, Dec 63) — unspellable schema identity for the
/// `FrozenCallable` payload of `FrozenType::Callable` (the fully-inferred
/// callable signature descriptor). Carries the ordered `params` array (each a
/// [`COMPTIME_FROZEN_PARAM_DESCRIPTOR_SCHEMA`] object) plus the return type's
/// frozen identity halves. Identities + typed descriptor data only — no
/// descriptor strings, no rendered type names (spec §3.1). The SOH prefix keeps
/// it unspellable so source can never forge a signature. NOT the legacy E5
/// `__ComptimeParamDescriptor` path.
pub const COMPTIME_FROZEN_CALLABLE_SCHEMA: &str = "\u{1}comptime:FrozenCallable";

/// ADR-009 B6 (Stage 2, Dec 63) — unspellable schema identity for one
/// signature-indexed positional `ParamDescriptor` in a [`FrozenCallable`].
/// Carries the parameter type's frozen identity halves, the `optional` flag,
/// and the `PassingMode` enum carrier (`Move` / `SharedBorrow` /
/// `ExclusiveBorrow`, derived from the parameter's borrow annotation). Parameter
/// NAMES are identity-insignificant and stay a freeze fact (hygienic
/// `param(#name)` resolution re-reads them from the freeze), never a runtime
/// string field. DISTINCT from the legacy `__ComptimeParamDescriptor` (a
/// string-typed E5 path — do NOT reuse).
pub const COMPTIME_FROZEN_PARAM_DESCRIPTOR_SCHEMA: &str = "\u{1}comptime:ParamDescriptor";

/// ADR-009 B7 (Stage 2, Dec 50/94) — unspellable schema identities for the four
/// composite `FrozenType` payloads and their element rows. Each carries only
/// typed descriptor data: catalog-ordinal variant ids on the wrapping enum plus
/// 128-bit child identity halves and typed nested objects — never a rendered
/// type-name string, never a string `.kind` field (Dec 50/94 required
/// rejection). The SOH prefix keeps every carrier unspellable so source can
/// never forge a composite descriptor.
///
/// `FrozenTuple` carries the ordered `elements` array (each a
/// [`COMPTIME_TUPLE_ELEMENT_SCHEMA`] object: positional index + element type
/// identity halves). `FrozenRecord` carries the normalized `fields` array (each
/// a [`COMPTIME_RECORD_FIELD_SCHEMA`] object: owner-bound hygienic member
/// identity halves + field type identity halves + `optional`). `FrozenReference`
/// carries the `mutable` flag + the referent type identity halves.
/// `FrozenUnion` carries the set `members` array (each a
/// [`COMPTIME_UNION_MEMBER_SCHEMA`] object: member type identity halves).
pub const COMPTIME_FROZEN_TUPLE_SCHEMA: &str = "\u{1}comptime:FrozenTuple";
/// See [`COMPTIME_FROZEN_TUPLE_SCHEMA`]. One positional tuple element.
pub const COMPTIME_TUPLE_ELEMENT_SCHEMA: &str = "\u{1}comptime:TupleElement";
/// See [`COMPTIME_FROZEN_TUPLE_SCHEMA`]. The normalized structural record.
pub const COMPTIME_FROZEN_RECORD_SCHEMA: &str = "\u{1}comptime:FrozenRecord";
/// See [`COMPTIME_FROZEN_TUPLE_SCHEMA`]. One normalized record field —
/// owner-bound member identity (never a source-name string, Dec 57).
pub const COMPTIME_RECORD_FIELD_SCHEMA: &str = "\u{1}comptime:RecordField";
/// See [`COMPTIME_FROZEN_TUPLE_SCHEMA`]. A reference type (`&T` / `&mut T`).
pub const COMPTIME_FROZEN_REFERENCE_SCHEMA: &str = "\u{1}comptime:FrozenReference";
/// See [`COMPTIME_FROZEN_TUPLE_SCHEMA`]. The normalized (deduped, byte-sorted)
/// union.
pub const COMPTIME_FROZEN_UNION_SCHEMA: &str = "\u{1}comptime:FrozenUnion";
/// See [`COMPTIME_FROZEN_TUPLE_SCHEMA`]. One union member type identity.
pub const COMPTIME_UNION_MEMBER_SCHEMA: &str = "\u{1}comptime:UnionMember";
/// ADR-009 B7 Slice 2 (Stage 2, Dec 50/94) — the `FrozenType::Parameter`
/// payload carrier (`TypeParamDescriptor<T>`): the type parameter's stable
/// base-fn-scoped frozen identity halves + a bound-set array. The bounds
/// element mirrors [`COMPTIME_FROZEN_ERASED_SCHEMA`] exactly — trait-reference
/// bound descriptors are ticket B2 territory, so the element is uninhabited
/// and the array is provably empty today (the honest "bounds where
/// representable" form, never an inference hole). The SOH prefix keeps it
/// unspellable so source can never forge a parameter descriptor.
pub const COMPTIME_FROZEN_PARAMETER_SCHEMA: &str = "\u{1}comptime:FrozenParameter";

/// ADR-009 B5 (Stage 2, Dec 55-59) — unspellable schema identities for the
/// nominal-shape descriptor family carried by `FrozenType::Nominal`. Each is a
/// typed descriptor carrier (owner-bound member identities, never source-name
/// strings — Dec 57); the SOH prefix keeps them unspellable so source can never
/// forge a shape. `FrozenNominal` carries the sealed `NominalShape` enum
/// (`shape`); the four shape variants carry the row structs below. NOT the
/// legacy E5 `__ComptimeFieldDescriptor` name-keyed path (do NOT reuse).
pub const COMPTIME_FROZEN_NOMINAL_SCHEMA: &str = "\u{1}comptime:FrozenNominal";
/// See [`COMPTIME_FROZEN_NOMINAL_SCHEMA`].
pub const COMPTIME_STRUCT_DESCRIPTOR_SCHEMA: &str = "\u{1}comptime:StructDescriptor";
/// See [`COMPTIME_FROZEN_NOMINAL_SCHEMA`].
pub const COMPTIME_ENUM_DESCRIPTOR_SCHEMA: &str = "\u{1}comptime:EnumDescriptor";
/// See [`COMPTIME_FROZEN_NOMINAL_SCHEMA`].
pub const COMPTIME_NEWTYPE_DESCRIPTOR_SCHEMA: &str = "\u{1}comptime:NewtypeDescriptor";
/// See [`COMPTIME_FROZEN_NOMINAL_SCHEMA`].
pub const COMPTIME_OPAQUE_TYPE_DESCRIPTOR_SCHEMA: &str = "\u{1}comptime:OpaqueTypeDescriptor";
/// See [`COMPTIME_FROZEN_NOMINAL_SCHEMA`]. One record field — owner-bound
/// member identity (`#f`), value-type frozen identity, initialization
/// disposition (Dec 57/59).
pub const COMPTIME_FIELD_DESCRIPTOR_SCHEMA: &str = "\u{1}comptime:FieldDescriptor";
/// See [`COMPTIME_FROZEN_NOMINAL_SCHEMA`]. One enum variant — owner-bound
/// member identity + payload arity.
pub const COMPTIME_VARIANT_DESCRIPTOR_SCHEMA: &str = "\u{1}comptime:VariantDescriptor";
/// See [`COMPTIME_FROZEN_NOMINAL_SCHEMA`]. One declaration-interface associated
/// constant — owner-bound member identity + value-type frozen identity (Dec 58).
pub const COMPTIME_ASSOCIATED_CONST_DESCRIPTOR_SCHEMA: &str =
    "\u{1}comptime:AssociatedConstDescriptor";

/// ADR-009 B5 (Stage 2, Dec 56): the `RepresentationAccess<T>` authority
/// capability. Complete nominal-shape reflection (`reflect_repr`) requires a
/// compiler-issued value of this schema, bound to the exact type identity `T`
/// it authorizes. The SOH-prefixed name cannot be spelled in Shape source, so
/// user code can never construct a lookalike capability; the schema-name-checked
/// decode in shape-vm's `comptime_builtins/type_reflection.rs` therefore blocks
/// forged authority structurally (the TraitRef/ImplRef precedent). Only the
/// compiler mints one — delivered to a declaration-attached annotation expand
/// hook as author consent (Dec 56).
pub const COMPTIME_REPRESENTATION_ACCESS_SCHEMA: &str = "\u{1}comptime:RepresentationAccess";

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

    // ADR-009 E5 CKPT-5 — the `.source` reparse-fallback FIELD is DELETED. Every
    // producer now stamps `identity_high`/`identity_low` with the
    // `FrozenTypeIdentity` (INVALID = {-1,-1} for an unstamped ref) and the
    // consumer (`type_annotation_from_string_or_type_ref_slot`) resolves
    // identity-only via `reconstruct_type_annotation` — there is no `.source`
    // spelling to reparse and no fallback arm in existence. `name`/`kind` remain
    // as the U02 corpus's spell/reflect-only fields (`field.type_ref.kind`,
    // derived from the type spelling at build time; NEVER reparsed into a type).
    // Every reader is name-keyed (`schema.get_field(name)`), so dropping the
    // middle `source` field — offsets are now name=0, kind=1, identity_high=2,
    // identity_low=3 — shifts no reader. Distinct from
    // `COMPTIME_FROZEN_TYPE_REF_SCHEMA` (below): that is the `type_ref(T)`
    // intrinsic's opaque frozen ref; THIS is the annotation-handler
    // `target.params[].type_ref` carrier the U02 corpus reads.
    let _comptime_type_ref = TypeSchemaBuilder::new("__ComptimeTypeRef")
        .string_field("name")
        .string_field("kind")
        .int_field("identity_high")
        .int_field("identity_low")
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

    // -- ADR-009 B1 S1: payload-bearing descriptor schemas -----------------
    //
    // The four `reflect()` descriptor schemas. All four are unspellable
    // (SOH-prefixed) so Shape source can never construct a lookalike
    // nominal carrier, and every one has a named arm in
    // `comptime_reflection::runtime_lift_rejection` — registered in the
    // SAME commit, so no schema ever exists without a lift wall. No schema
    // exposes a string `kind` field or a nullable category field
    // (Dec 50/94 required rejection).

    // `FrozenType`: the sealed indexed sum. ONLY the enabled payload
    // variants are declared (no forgeable stub variants for the seven
    // pending categories); variant ids are the Dec 50/94 catalog ORDINALS
    // (Primitive=0, Never=1, Erased=9) so later B tickets extend the enum
    // without renumbering (comptime-ABI stability, spec §3.3).
    let _comptime_frozen_type = registry.register_enum_scoped(
        COMPTIME_FROZEN_TYPE_SCHEMA,
        crate::comptime_reflection::FROZEN_TYPE_ENABLED_PAYLOAD_CATEGORIES
            .into_iter()
            .map(|category| {
                EnumVariantInfo::new(category.variant_name(), category.catalog_ordinal(), 1)
            })
            .collect(),
    );

    // `FrozenPrimitive`: the sealed sub-algebra, generated from the shared
    // runtime catalog — same names, same order, same typed width/domain
    // payload arities. The sub-algebra is complete (all members land with
    // B1), so declaration-order ids are canonical.
    let _comptime_frozen_primitive = registry.register_enum_scoped(
        COMPTIME_FROZEN_PRIMITIVE_SCHEMA,
        crate::comptime_reflection::FROZEN_PRIMITIVE_VARIANTS
            .iter()
            .enumerate()
            .map(|(id, variant)| {
                EnumVariantInfo::new(variant.name, id as u16, variant.payload_arity)
            })
            .collect(),
    );

    // ADR-009 B1 S2 — width-domain enum carriers for the `FrozenPrimitive`
    // integer/float family payloads, generated from the shared runtime
    // catalog (`IntegerWidth::ALL` / `FloatWidth::ALL` + `variant_name`).
    // Spellable names, following the `FrozenTypeCategory` precedent (user
    // comptime code matches their variants); each has its own named
    // `runtime_lift_rejection` arm registered in the same commit.
    let _integer_width = registry.register_enum_scoped(
        crate::comptime_reflection::INTEGER_WIDTH_SCHEMA_NAME,
        crate::comptime_reflection::IntegerWidth::ALL
            .into_iter()
            .enumerate()
            .map(|(id, width)| EnumVariantInfo::new(width.variant_name(), id as u16, 0))
            .collect(),
    );
    let _float_width = registry.register_enum_scoped(
        crate::comptime_reflection::FLOAT_WIDTH_SCHEMA_NAME,
        crate::comptime_reflection::FloatWidth::ALL
            .into_iter()
            .enumerate()
            .map(|(id, width)| EnumVariantInfo::new(width.variant_name(), id as u16, 0))
            .collect(),
    );

    // `FrozenNever`: zero-field marker descriptor.
    let _comptime_frozen_never =
        TypeSchemaBuilder::new(COMPTIME_FROZEN_NEVER_SCHEMA).register(registry);

    // `FrozenErased`: the bound-set array only. Bounds are reachable today
    // solely as the empty set (`any`); trait-bound elements arrive with
    // A2/B2 — the element FieldType is informational (heap_mask + the
    // parallel field-kind track drive reads), matching the
    // `__ComptimeFieldDescriptor.annotations` precedent.
    let _comptime_frozen_erased = TypeSchemaBuilder::new(COMPTIME_FROZEN_ERASED_SCHEMA)
        .array_field(
            "bounds",
            crate::type_schema::any_migration::bounds_array_element(),
        )
        .register(registry);

    // ADR-009 (ticket B2, slice S3): reserved opaque carriers for
    // compiler-issued trait identities and implementation evidence (Dec 49).
    // Identity halves only — no name/kind text fields, so no name-based
    // lookup survives into the carriers. The SOH-prefixed names cannot be
    // spelled in Shape source: source code can never construct a lookalike
    // carrier (the schema-name-checked decode in shape-vm's
    // `comptime_builtins/trait_evidence.rs` therefore blocks forged evidence
    // structurally).
    let _comptime_frozen_trait_ref = TypeSchemaBuilder::new(COMPTIME_FROZEN_TRAIT_REF_SCHEMA)
        .int_field("identity_high")
        .int_field("identity_low")
        .register(registry);

    // `ImplRef` ties evidence to the exact `(trait, type)` identity pair AND
    // to the exact (possibly named) impl: the impl's canonical identity
    // (`impl:{trait}:{type}:{impl_name_or_default}` descriptor hash) rides
    // in the carrier so it can enter generated-artifact descriptor
    // fingerprints (Dec 49).
    let _comptime_frozen_impl_ref = TypeSchemaBuilder::new(COMPTIME_FROZEN_IMPL_REF_SCHEMA)
        .int_field("trait_identity_high")
        .int_field("trait_identity_low")
        .int_field("type_identity_high")
        .int_field("type_identity_low")
        .int_field("impl_identity_high")
        .int_field("impl_identity_low")
        .register(registry);

    // -- ADR-009 B4 (Stage 2, Dec 54): uniform nominal application ----------
    //
    // `ParamKind` is the sealed shared-catalog vocabulary (`Type` | `Const`)
    // the freeze's per-constructor kind vector is built from and `.apply(...)`
    // checks each supplied argument against. Registered here as a spellable
    // enum (the `FrozenTypeCategory` precedent) so LSP completes its variants
    // and the value carrier route stays walled by its own
    // `runtime_lift_rejection` arm — registered in the SAME commit as this
    // schema. Generated from the shared runtime catalog (`ParamKind::ALL`);
    // no second hand-written kind list.
    let _param_kind = registry.register_enum_scoped(
        crate::comptime_reflection::PARAM_KIND_SCHEMA_NAME,
        crate::comptime_reflection::ParamKind::ALL
            .into_iter()
            .enumerate()
            .map(|(id, kind)| EnumVariantInfo::new(kind.variant_name(), id as u16, 0))
            .collect(),
    );

    // `TypeConstructorRef`: the frozen nominal HEAD identity halves only. The
    // ordered parameter kinds are re-read from the freeze at `.apply(...)`
    // (single param-kind authority), never a second table baked into the
    // carrier. SOH-prefixed name + schema-name-checked decode blocks forged
    // constructors structurally (the TraitRef/ImplRef precedent).
    let _comptime_type_constructor_ref =
        TypeSchemaBuilder::new(COMPTIME_FROZEN_TYPE_CONSTRUCTOR_REF_SCHEMA)
            .int_field("identity_high")
            .int_field("identity_low")
            .register(registry);

    // `AppliedType`: applied identity halves (identity-EQUAL to the A2
    // `type_ref(Head<Args>)` spelling), head identity halves, and the ordered
    // argument identities as an interleaved `high, low, …` int array. Enough
    // to `refine(...)` and `type_argument(i)` from stored identities, never by
    // inverting the one-way SHA-256 hash. No descriptor strings.
    let _comptime_applied_type = TypeSchemaBuilder::new(COMPTIME_APPLIED_TYPE_SCHEMA)
        .int_field("identity_high")
        .int_field("identity_low")
        .int_field("head_identity_high")
        .int_field("head_identity_low")
        .array_field("arg_identities", FieldType::I64)
        .register(registry);

    // -- ADR-009 B6 (Stage 2, Dec 63): callable signature descriptors ---------
    //
    // `PassingMode` is the sealed shared-catalog mode axis (`Move` |
    // `SharedBorrow` | `ExclusiveBorrow`) derived at freeze time from a
    // parameter's borrow annotation. Registered as a spellable enum (the
    // `FrozenTypeCategory` / `ParamKind` precedent) so LSP completes its
    // variants; walled by its own `runtime_lift_rejection` arm registered in
    // the SAME commit. Generated from the shared runtime catalog
    // (`PassingMode::ALL`); no second hand-written mode list.
    let _passing_mode = registry.register_enum_scoped(
        crate::comptime_reflection::PASSING_MODE_SCHEMA_NAME,
        crate::comptime_reflection::PassingMode::ALL
            .into_iter()
            .enumerate()
            .map(|(id, mode)| EnumVariantInfo::new(mode.variant_name(), id as u16, 0))
            .collect(),
    );

    // `ParamDescriptor`: one signature-indexed positional parameter. Type
    // identity halves + the optional flag + the `PassingMode` enum carrier.
    // Registered before `FrozenCallable`, which references it as its array
    // element. No string type-name / kind field (Dec 50/94 required rejection);
    // parameter names stay a freeze fact, never a runtime string.
    let _comptime_frozen_param_descriptor =
        TypeSchemaBuilder::new(COMPTIME_FROZEN_PARAM_DESCRIPTOR_SCHEMA)
            .int_field("type_identity_high")
            .int_field("type_identity_low")
            .bool_field("optional")
            .object_field("mode", crate::comptime_reflection::PASSING_MODE_SCHEMA_NAME)
            .register(registry);

    // `FrozenCallable`: the fully-inferred callable signature. Ordered `params`
    // array (each a `ParamDescriptor` object) + the return type's frozen
    // identity halves. Unlike `FrozenErased.bounds` (an uninhabited/empty set,
    // whitelisted `FieldType::Any`), the params element is inhabited, so it is
    // the TYPED `ParamDescriptor` object element — no `FieldType::Any`, no
    // post-inference whitelist entry needed.
    let _comptime_frozen_callable = TypeSchemaBuilder::new(COMPTIME_FROZEN_CALLABLE_SCHEMA)
        .array_field(
            "params",
            FieldType::Object(COMPTIME_FROZEN_PARAM_DESCRIPTOR_SCHEMA.to_string()),
        )
        .int_field("returns_identity_high")
        .int_field("returns_identity_low")
        .register(registry);

    generated_capture::register(registry);

    // -- ADR-009 B5 (Stage 2, Dec 55-59): nominal-shape descriptors -----------
    //
    // `NominalShape` is the sealed shared-catalog declaration-shape axis
    // (`Struct` | `Enum` | `Newtype` | `Opaque`) that `FrozenNominal.shape()`
    // projects. Registered as a spellable enum (the `PassingMode` precedent) so
    // LSP completes its variants; walled by its own `runtime_lift_rejection`
    // arm registered in the SAME commit. Each shape variant carries its typed
    // row-struct descriptor (payload arity 1). Generated from the shared runtime
    // catalog (`NominalShape::ALL`); no second hand-written shape list. The
    // variant→descriptor payload mapping is applied by the mini-VM injected
    // model (`frozen_type_payload_model_items`); here the value-carrier enum
    // records the arity so `__payload_0` is a stored slot.
    let _nominal_shape = registry.register_enum_scoped(
        crate::comptime_reflection::NOMINAL_SHAPE_SCHEMA_NAME,
        crate::comptime_reflection::NominalShape::ALL
            .into_iter()
            .enumerate()
            .map(|(id, shape)| EnumVariantInfo::new(shape.variant_name(), id as u16, 1))
            .collect(),
    );

    // `FieldInitialization`: the sealed Dec 59 disposition (`Required` |
    // `Defaulted`). Spellable enum, walled by its own lift arm.
    let _field_initialization = registry.register_enum_scoped(
        crate::comptime_reflection::FIELD_INITIALIZATION_SCHEMA_NAME,
        crate::comptime_reflection::FieldInitialization::ALL
            .into_iter()
            .enumerate()
            .map(|(id, init)| EnumVariantInfo::new(init.variant_name(), id as u16, 0))
            .collect(),
    );

    // `FieldDescriptor`: one record field — owner identity halves + owner-bound
    // member identity (`#f`, Dec 57: NEVER a source-name string) + value-type
    // frozen identity halves + the `FieldInitialization` enum carrier.
    // Registered before `StructDescriptor`, which references it as its array
    // element.
    let _comptime_field_descriptor = TypeSchemaBuilder::new(COMPTIME_FIELD_DESCRIPTOR_SCHEMA)
        .int_field("owner_identity_high")
        .int_field("owner_identity_low")
        .int_field("member_high")
        .int_field("member_low")
        .int_field("type_identity_high")
        .int_field("type_identity_low")
        .object_field(
            "initialization",
            crate::comptime_reflection::FIELD_INITIALIZATION_SCHEMA_NAME,
        )
        .register(registry);

    // `VariantDescriptor`: one enum variant — owner identity halves +
    // owner-bound member identity + payload arity.
    let _comptime_variant_descriptor = TypeSchemaBuilder::new(COMPTIME_VARIANT_DESCRIPTOR_SCHEMA)
        .int_field("owner_identity_high")
        .int_field("owner_identity_low")
        .int_field("member_high")
        .int_field("member_low")
        .int_field("payload_arity")
        .register(registry);

    // `AssociatedConstDescriptor`: one declaration-interface associated
    // constant (Dec 58) — owner identity halves + owner-bound member identity +
    // value-type frozen identity halves. No runtime slot (a const member is not
    // a field, Dec 58).
    let _comptime_associated_const_descriptor =
        TypeSchemaBuilder::new(COMPTIME_ASSOCIATED_CONST_DESCRIPTOR_SCHEMA)
            .int_field("owner_identity_high")
            .int_field("owner_identity_low")
            .int_field("member_high")
            .int_field("member_low")
            .int_field("type_identity_high")
            .int_field("type_identity_low")
            .register(registry);

    // `StructDescriptor`: owner identity halves + the runtime field count + the
    // ordered `fields` array (each a `FieldDescriptor` object — a TYPED element,
    // no `FieldType::Any`, so no post-inference whitelist entry needed).
    let _comptime_struct_descriptor = TypeSchemaBuilder::new(COMPTIME_STRUCT_DESCRIPTOR_SCHEMA)
        .int_field("owner_identity_high")
        .int_field("owner_identity_low")
        .int_field("field_count")
        .array_field(
            "fields",
            FieldType::Object(COMPTIME_FIELD_DESCRIPTOR_SCHEMA.to_string()),
        )
        .register(registry);

    // `EnumDescriptor`: owner identity halves + the variant count + the ordered
    // `variants` array (each a `VariantDescriptor` object).
    let _comptime_enum_descriptor = TypeSchemaBuilder::new(COMPTIME_ENUM_DESCRIPTOR_SCHEMA)
        .int_field("owner_identity_high")
        .int_field("owner_identity_low")
        .int_field("variant_count")
        .array_field(
            "variants",
            FieldType::Object(COMPTIME_VARIANT_DESCRIPTOR_SCHEMA.to_string()),
        )
        .register(registry);

    // `NewtypeDescriptor`: owner identity halves + the single inner type's
    // frozen identity halves (the wrapped `U`).
    let _comptime_newtype_descriptor = TypeSchemaBuilder::new(COMPTIME_NEWTYPE_DESCRIPTOR_SCHEMA)
        .int_field("owner_identity_high")
        .int_field("owner_identity_low")
        .int_field("inner_identity_high")
        .int_field("inner_identity_low")
        .register(registry);

    // `OpaqueTypeDescriptor`: owner identity halves only — a semantically
    // non-decomposable nominal exposes its identity, never a representation.
    let _comptime_opaque_type_descriptor =
        TypeSchemaBuilder::new(COMPTIME_OPAQUE_TYPE_DESCRIPTOR_SCHEMA)
            .int_field("owner_identity_high")
            .int_field("owner_identity_low")
            .register(registry);

    // `FrozenNominal`: the payload of `FrozenType::Nominal` — the sealed
    // `NominalShape` enum carrier only (`.shape()` projects it). Field/variant
    // arrays live inside the shape descriptors above, never a partial
    // representation on the wrapper itself (Dec 56).
    let _comptime_frozen_nominal = TypeSchemaBuilder::new(COMPTIME_FROZEN_NOMINAL_SCHEMA)
        .object_field(
            "shape",
            crate::comptime_reflection::NOMINAL_SHAPE_SCHEMA_NAME,
        )
        .register(registry);

    // ADR-009 B5 (Stage 2, Dec 56): `RepresentationAccess<T>` — the authority
    // capability gating `reflect_repr`. Identity halves only — the exact frozen
    // TYPE identity `T` this capability authorizes complete-shape reflection
    // over. No name/kind text: authority is tied to a compiler-issued identity,
    // never a source spelling. The SOH-prefixed name blocks a forged lookalike
    // structurally (the TypeConstructorRef/TraitRef precedent). Its lift wall
    // arm is registered in the SAME commit (`comptime_reflection.rs`).
    let _comptime_representation_access =
        TypeSchemaBuilder::new(COMPTIME_REPRESENTATION_ACCESS_SCHEMA)
            .int_field("identity_high")
            .int_field("identity_low")
            .register(registry);

    // -- ADR-009 B7 (Stage 2, Dec 50/94): composite payload descriptors -------
    //
    // The four composite `FrozenType` payloads and their typed element rows.
    // Element schemas register BEFORE the array-holding wrappers that reference
    // them. Every field is a typed identity half or a bool — no string type-name
    // / kind field (Dec 50/94 required rejection). Each carrier is walled by its
    // own `runtime_lift_rejection` arm registered in the SAME commit.

    // `TupleElement`: one positional element — the position index + the element
    // type's frozen identity halves.
    let _comptime_tuple_element = TypeSchemaBuilder::new(COMPTIME_TUPLE_ELEMENT_SCHEMA)
        .int_field("index")
        .int_field("type_identity_high")
        .int_field("type_identity_low")
        .register(registry);

    // `FrozenTuple`: the ordered `elements` array (each a `TupleElement` object —
    // a TYPED element, so no `FieldType::Any`, no post-inference whitelist).
    let _comptime_frozen_tuple = TypeSchemaBuilder::new(COMPTIME_FROZEN_TUPLE_SCHEMA)
        .array_field(
            "elements",
            FieldType::Object(COMPTIME_TUPLE_ELEMENT_SCHEMA.to_string()),
        )
        .register(registry);

    // `RecordField`: one normalized record field — owner-bound hygienic member
    // identity halves (`#f`, Dec 57: the identity/dedup-bearing handle is NEVER
    // a source-name string), the field type's frozen identity halves, and the
    // `optional` flag. ADR-009 E5 CKPT-3 (B2 in-scope): `name` is the field's
    // plain source name, surfaced ADDITIVELY as a Dec-55-class spell/reflect-only
    // comptime-ABI field (mirrors how `__ComptimeFieldDescriptor.name` surfaces a
    // struct field name). It is a presentation fact layered BESIDE the member
    // identity — the CKPT-0 binding invariant keeps the record identity + member
    // strings byte-identical, so this adds no information to the identity algebra.
    let _comptime_record_field = TypeSchemaBuilder::new(COMPTIME_RECORD_FIELD_SCHEMA)
        .int_field("member_high")
        .int_field("member_low")
        .int_field("type_identity_high")
        .int_field("type_identity_low")
        .bool_field("optional")
        .string_field("name")
        .register(registry);

    // `FrozenRecord`: the normalized structural record — the `fields` array
    // (byte-sorted by member, each a `RecordField` object).
    let _comptime_frozen_record = TypeSchemaBuilder::new(COMPTIME_FROZEN_RECORD_SCHEMA)
        .array_field(
            "fields",
            FieldType::Object(COMPTIME_RECORD_FIELD_SCHEMA.to_string()),
        )
        .register(registry);

    // `FrozenReference`: the `mutable` flag (`&T` vs `&mut T`) + the referent
    // type's frozen identity halves.
    let _comptime_frozen_reference = TypeSchemaBuilder::new(COMPTIME_FROZEN_REFERENCE_SCHEMA)
        .bool_field("mutable")
        .int_field("referent_identity_high")
        .int_field("referent_identity_low")
        .register(registry);

    // `UnionMember`: one union member — its frozen identity halves.
    let _comptime_union_member = TypeSchemaBuilder::new(COMPTIME_UNION_MEMBER_SCHEMA)
        .int_field("type_identity_high")
        .int_field("type_identity_low")
        .register(registry);

    // `FrozenUnion`: the set `members` array (deduped, byte-sorted, each a
    // `UnionMember` object — a singleton union coalesces to its member upstream,
    // so a `FrozenUnion` always carries ≥2 members).
    let _comptime_frozen_union = TypeSchemaBuilder::new(COMPTIME_FROZEN_UNION_SCHEMA)
        .array_field(
            "members",
            FieldType::Object(COMPTIME_UNION_MEMBER_SCHEMA.to_string()),
        )
        .register(registry);

    // -- ADR-009 B7 Slice 2 (Stage 2, Dec 50/94): the Parameter payload --------
    //
    // `FrozenParameter`: the payload of `FrozenType::Parameter` — the type
    // parameter's stable base-fn-scoped frozen identity halves + the bound-set
    // array. Bounds mirror `FrozenErased.bounds` exactly: trait-reference bound
    // descriptors are ticket B2 territory (the element is uninhabited), so the
    // array is provably empty today — the honest "bounds where representable"
    // form, never an inference hole. The element `FieldType` is informational
    // (heap_mask + the parallel field-kind track drive reads), matching the
    // `FrozenErased.bounds` / `__ComptimeFieldDescriptor.annotations` precedent.
    // Walled by its own `runtime_lift_rejection` arm registered in the SAME
    // commit. No string type-name / kind field (Dec 50/94 required rejection).
    let _comptime_frozen_parameter = TypeSchemaBuilder::new(COMPTIME_FROZEN_PARAMETER_SCHEMA)
        .int_field("identity_high")
        .int_field("identity_low")
        .array_field(
            "bounds",
            crate::type_schema::any_migration::bounds_array_element(),
        )
        .register(registry);

    // ADR-009 E2 #18 (slice 2): the typed `item_fn` carrier (E2-D10). This schema
    // carries ONLY an opaque `index` handle into the driver's thread-local
    // `CheckedItem` store — the built AST `Item` lives compiler-side, never
    // re-encoded into fields (the FrozenTypeRef opaque-identity pattern). (Slice 5
    // deleted the legacy `__ComptimeItemFragment` sentinel schema this replaced —
    // it encoded the whole declaration into `kind`/`name`/`return_type`/`literal_*`
    // fields.)
    let _comptime_checked_item = TypeSchemaBuilder::new("__CheckedItem")
        .int_field("index")
        .register(registry);

    let _comptime_field_descriptor = TypeSchemaBuilder::new("__ComptimeFieldDescriptor")
        .string_field("name")
        .string_field("type")
        .array_field(
            "annotations",
            crate::type_schema::any_migration::class_c_array_field_element(),
        )
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
        .array_field(
            "args",
            crate::type_schema::any_migration::class_c_array_field_element(),
        )
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
        .array_field(
            "fields",
            crate::type_schema::any_migration::class_c_array_field_element(),
        )
        .array_field(
            "params",
            crate::type_schema::any_migration::class_c_array_field_element(),
        )
        .option_string_field("return_type")
        .object_field("return_type_ref", "__ComptimeTypeRef")
        .array_field(
            "annotations",
            crate::type_schema::any_migration::class_c_array_field_element(),
        )
        .array_field(
            "captures",
            crate::type_schema::any_migration::class_c_array_field_element(),
        )
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

    // ADR-009 C3 #14 (slice 2): the typed hook-template + capture-binding
    // carriers — the E2 `__CheckedItem` opaque-index pattern verbatim. Each
    // schema carries ONLY an opaque `index` handle into the driver's
    // thread-local store (`comptime_builtins.rs`: `COMPTIME_HOOK_TEMPLATES` /
    // `COMPTIME_CAPTURE_BINDINGS`); the checked template / lifted capture
    // value live compiler-side, never re-encoded into fields. Appended AFTER
    // every pre-existing registration so no order-stable builtin schema id
    // shifts.
    let _comptime_checked_template = TypeSchemaBuilder::new("__CheckedTemplate")
        .int_field("index")
        .register(registry);
    let _comptime_capture_binding = TypeSchemaBuilder::new("__CaptureBinding")
        .int_field("index")
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
        assert!(registry.has_type("__ComptimeTypeRef"));
        assert!(registry.has_type(COMPTIME_FROZEN_TYPE_REF_SCHEMA));
        assert!(registry.has_type(COMPTIME_FROZEN_TRAIT_REF_SCHEMA));
        assert!(registry.has_type(COMPTIME_FROZEN_IMPL_REF_SCHEMA));
        assert!(registry.has_type("FrozenTypeCategory"));

        // ADR-009 B4 (Stage 2, Dec 54): uniform nominal-application carriers +
        // the ParamKind vocabulary.
        assert!(registry.has_type(COMPTIME_FROZEN_TYPE_CONSTRUCTOR_REF_SCHEMA));
        assert!(registry.has_type(COMPTIME_APPLIED_TYPE_SCHEMA));
        assert!(registry.has_type(crate::comptime_reflection::PARAM_KIND_SCHEMA_NAME));

        let constructor = registry
            .get(COMPTIME_FROZEN_TYPE_CONSTRUCTOR_REF_SCHEMA)
            .unwrap();
        assert_eq!(constructor.field_count(), 2);
        assert!(constructor.get_field("identity_high").is_some());
        assert!(constructor.get_field("identity_low").is_some());

        let applied = registry.get(COMPTIME_APPLIED_TYPE_SCHEMA).unwrap();
        assert_eq!(applied.field_count(), 5);
        assert!(applied.get_field("head_identity_high").is_some());
        assert!(applied.get_field("arg_identities").is_some());

        // ParamKind: Type | Const | Effect (ADR-014 §8.3 added the binder).
        let param_kind = registry
            .get(crate::comptime_reflection::PARAM_KIND_SCHEMA_NAME)
            .unwrap();
        assert!(param_kind.variant_id("Type").is_some());
        assert!(param_kind.variant_id("Const").is_some());
        assert!(param_kind.variant_id("Effect").is_some());

        // ADR-009 B6 (Stage 2, Dec 63): the callable signature descriptor
        // carriers + the PassingMode mode axis.
        assert!(registry.has_type(COMPTIME_FROZEN_CALLABLE_SCHEMA));
        assert!(registry.has_type(COMPTIME_FROZEN_PARAM_DESCRIPTOR_SCHEMA));
        assert!(registry.has_type(crate::comptime_reflection::PASSING_MODE_SCHEMA_NAME));

        let callable = registry.get(COMPTIME_FROZEN_CALLABLE_SCHEMA).unwrap();
        assert_eq!(callable.field_count(), 3);
        assert!(callable.get_field("params").is_some());
        assert!(callable.get_field("returns_identity_high").is_some());
        assert!(callable.get_field("returns_identity_low").is_some());

        let param_descriptor = registry
            .get(COMPTIME_FROZEN_PARAM_DESCRIPTOR_SCHEMA)
            .unwrap();
        assert_eq!(param_descriptor.field_count(), 4);
        assert!(param_descriptor.get_field("type_identity_high").is_some());
        assert!(param_descriptor.get_field("type_identity_low").is_some());
        assert!(param_descriptor.get_field("optional").is_some());
        assert!(param_descriptor.get_field("mode").is_some());

        // PassingMode is a three-variant enum (the ADR mode axis).
        let passing_mode = registry
            .get(crate::comptime_reflection::PASSING_MODE_SCHEMA_NAME)
            .unwrap();
        assert!(passing_mode.variant_id("Move").is_some());
        assert!(passing_mode.variant_id("SharedBorrow").is_some());
        assert!(passing_mode.variant_id("ExclusiveBorrow").is_some());

        // ADR-009 B5 (Stage 2, Dec 55-59): the nominal-shape descriptor family
        // + the NominalShape declaration-shape axis + FieldInitialization.
        assert!(registry.has_type(COMPTIME_FROZEN_NOMINAL_SCHEMA));
        assert!(registry.has_type(COMPTIME_STRUCT_DESCRIPTOR_SCHEMA));
        assert!(registry.has_type(COMPTIME_ENUM_DESCRIPTOR_SCHEMA));
        assert!(registry.has_type(COMPTIME_NEWTYPE_DESCRIPTOR_SCHEMA));
        assert!(registry.has_type(COMPTIME_OPAQUE_TYPE_DESCRIPTOR_SCHEMA));
        assert!(registry.has_type(COMPTIME_FIELD_DESCRIPTOR_SCHEMA));
        assert!(registry.has_type(COMPTIME_VARIANT_DESCRIPTOR_SCHEMA));
        assert!(registry.has_type(COMPTIME_ASSOCIATED_CONST_DESCRIPTOR_SCHEMA));
        assert!(registry.has_type(crate::comptime_reflection::NOMINAL_SHAPE_SCHEMA_NAME));
        assert!(registry.has_type(crate::comptime_reflection::FIELD_INITIALIZATION_SCHEMA_NAME));

        // NominalShape is a four-variant enum (the sealed declaration shapes).
        let nominal_shape = registry
            .get(crate::comptime_reflection::NOMINAL_SHAPE_SCHEMA_NAME)
            .unwrap();
        for variant in ["Struct", "Enum", "Newtype", "Opaque"] {
            assert!(nominal_shape.variant_id(variant).is_some(), "{variant}");
        }
        let frozen_nominal = registry.get(COMPTIME_FROZEN_NOMINAL_SCHEMA).unwrap();
        assert_eq!(frozen_nominal.field_count(), 1);
        assert!(frozen_nominal.get_field("shape").is_some());
        let struct_descriptor = registry.get(COMPTIME_STRUCT_DESCRIPTOR_SCHEMA).unwrap();
        assert!(struct_descriptor.get_field("fields").is_some());
        assert!(struct_descriptor.get_field("field_count").is_some());
        let field_descriptor = registry.get(COMPTIME_FIELD_DESCRIPTOR_SCHEMA).unwrap();
        assert!(field_descriptor.get_field("member_high").is_some());
        assert!(field_descriptor.get_field("initialization").is_some());
        // Dec 57 rejection: no source-name string field on the field descriptor.
        assert!(field_descriptor.get_field("name").is_none());

        // ADR-009 B5 (Stage 2, Dec 56): the RepresentationAccess authority
        // capability — identity halves only, no name/kind text.
        let representation_access = registry.get(COMPTIME_REPRESENTATION_ACCESS_SCHEMA).unwrap();
        assert_eq!(representation_access.field_count(), 2);
        assert!(representation_access.get_field("identity_high").is_some());
        assert!(representation_access.get_field("identity_low").is_some());
        assert!(representation_access.get_field("name").is_none());
        assert!(representation_access.get_field("kind").is_none());

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
