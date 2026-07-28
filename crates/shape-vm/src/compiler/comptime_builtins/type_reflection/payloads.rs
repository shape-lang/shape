//! ADR-009 B1 S2 — payload descriptors + heap-value builders for the sealed
//! `FrozenType` sum returned by `reflect(TypeRef<T>)` (Dec 50/94). // ADR-009
//!
//! The compiler-internal [`FrozenPayloadDescriptor`] is the query-API face
//! (`SemanticFreeze::payload_of` / `FreezeOverlay::payload_of` — the ONE
//! shared query surface, spec §4.1); [`build_frozen_type_heap_value`] lowers
//! it to the comptime value carrier: nested typed objects against the
//! unspellable descriptor schemas registered in `builtin_schemas.rs`
//! (`FrozenType` / `FrozenPrimitive` / `FrozenNever` / `FrozenErased`) plus
//! the width-domain enum carriers (`IntegerWidth` / `FloatWidth`).
//!
//! Payloads carry typed descriptor data ONLY — variant ordinals and nested
//! typed objects, never rendered type-name strings, never a string `kind`
//! field (Dec 50/94 required rejection). Reflecting a category whose payload
//! ticket has not landed is the named R1 per-category rejection produced by
//! [`pending_payload_rejection`] — never a partial descriptor.

use crate::compiler::comptime_builtins::semantic_freeze::FreezeOverlay;

use super::{FrozenTypeCategory, FrozenTypeIdentity};
use shape_runtime::comptime_reflection::{
    FIELD_INITIALIZATION_SCHEMA_NAME, FLOAT_WIDTH_SCHEMA_NAME, FieldInitialization,
    FrozenPrimitive, INTEGER_WIDTH_SCHEMA_NAME, NOMINAL_SHAPE_SCHEMA_NAME, NominalShape,
    PASSING_MODE_SCHEMA_NAME, PassingMode,
};
use shape_runtime::type_schema::builtin_schemas::{
    COMPTIME_ENUM_DESCRIPTOR_SCHEMA, COMPTIME_FIELD_DESCRIPTOR_SCHEMA,
    COMPTIME_FROZEN_CALLABLE_SCHEMA, COMPTIME_FROZEN_ERASED_SCHEMA, COMPTIME_FROZEN_NEVER_SCHEMA,
    COMPTIME_FROZEN_NOMINAL_SCHEMA, COMPTIME_FROZEN_PARAM_DESCRIPTOR_SCHEMA,
    COMPTIME_FROZEN_PARAMETER_SCHEMA, COMPTIME_FROZEN_PRIMITIVE_SCHEMA,
    COMPTIME_FROZEN_RECORD_SCHEMA, COMPTIME_FROZEN_REFERENCE_SCHEMA, COMPTIME_FROZEN_TUPLE_SCHEMA,
    COMPTIME_FROZEN_TYPE_SCHEMA, COMPTIME_FROZEN_UNION_SCHEMA, COMPTIME_NEWTYPE_DESCRIPTOR_SCHEMA,
    COMPTIME_OPAQUE_TYPE_DESCRIPTOR_SCHEMA, COMPTIME_RECORD_FIELD_SCHEMA,
    COMPTIME_STRUCT_DESCRIPTOR_SCHEMA, COMPTIME_TUPLE_ELEMENT_SCHEMA, COMPTIME_UNION_MEMBER_SCHEMA,
    COMPTIME_VARIANT_DESCRIPTOR_SCHEMA,
};
use shape_runtime::type_schema::{current_registry, typed_object_for_named_schema};
use shape_value::heap_value::{HeapKind, HeapValue, TypedObjectStorage};
use shape_value::v2::typed_array::{ELEM_TYPE_TYPED_OBJECT, TypedArray, stamp_elem_type};
use shape_value::{KindedSlot, NativeKind, ValueSlot};

/// Bound-set element carried by [`FrozenPayloadDescriptor::Erased`].
///
/// Ticket A2 made `dyn Trait` / trait-intersection spellings reachable
/// (they classify as the ENABLED Erased category), but their bound-set
/// elements are the trait-reference descriptors that land with ticket B2 —
/// so this element type is deliberately uninhabited: a non-empty bound set
/// is unrepresentable, which is the structural form of "no partial
/// descriptors" (spec §3.1). The ONLY erased identity whose payload query
/// succeeds is the base-frozen `any` leaf (the complete AND empty bound
/// set); every bounded erased identity is the named
/// [`bounded_erased_payload_rejection`] until B2 retypes this element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrozenErasedBound {}

/// ADR-009 B6 (Stage 2, Dec 63) — one signature-indexed positional parameter of
/// a [`FrozenPayloadDescriptor::Callable`] signature descriptor
/// (`ParamDescriptor<Sig, I, T, Mode>`).
///
/// The full structural detail the one-way SHA-256 canonical identity CANNOT
/// recover: the parameter NAME (identity-insignificant, kept for hygienic
/// `param(#name)` resolution) and the [`PassingMode`] (not part of the identity
/// string — reconstructed from the freeze's preserved structural descriptor).
/// `type_identity` is the parameter's VALUE type (the referent when the
/// parameter is borrowed; the mode carries the borrow), and `optional` marks a
/// trailing-optional parameter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParamDescriptor {
    pub(crate) name: Option<String>,
    pub(crate) type_identity: FrozenTypeIdentity,
    pub(crate) optional: bool,
    pub(crate) mode: PassingMode,
}

/// ADR-009 B6 — the ordered structural descriptor of a callable signature: the
/// positional parameters (in signature order) and the return type's frozen
/// identity. Threaded from the canonicalizer's `Function` arm through the
/// freeze's widened composite memo so [`FrozenPayloadDescriptor::Callable`] can
/// be reconstructed WITHOUT inverting the identity hash (which drops names and
/// modes by design).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CallableDescriptor {
    pub(crate) params: Vec<ParamDescriptor>,
    pub(crate) returns: FrozenTypeIdentity,
}

/// ADR-009 B5 (Stage 2, Dec 57) — one record field of a
/// [`NominalDescriptor::Struct`]. Carries the owner-bound HYGIENIC member
/// identity (`#f`, a stable opaque identity minted from `member:{owner}:{name}`
/// — NEVER the source-name string, Dec 57 rejection R1/R3), the field's VALUE
/// type frozen identity, and its [`FieldInitialization`] disposition. Field
/// names are identity-insignificant and stay a freeze fact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NominalFieldDescriptor {
    pub(crate) member: FrozenTypeIdentity,
    pub(crate) type_identity: FrozenTypeIdentity,
    pub(crate) initialization: FieldInitialization,
}

/// ADR-009 B5 (Stage 2, Dec 57) — one variant of a [`NominalDescriptor::Enum`]:
/// the owner-bound hygienic member identity + the payload arity (Unit=0,
/// Tuple(n)/Struct(n)=n).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NominalVariantDescriptor {
    pub(crate) member: FrozenTypeIdentity,
    pub(crate) payload_arity: u16,
}

/// ADR-009 B5 (Stage 2, Dec 55/57/58) — the sealed nominal declaration shape of
/// a frozen NOMINAL identity, reconstructed from the freeze's struct/enum inputs
/// (never re-derived). The `owner` is the reflected nominal's frozen identity;
/// generic substitution precedes issuance (Dec 55 — the applied identity's
/// substituted field types, threaded via B4 applied identities in a later
/// slice). Deliberately NO `Default`/partial constructor (rejection-matrix row
/// R8) — every value is fully populated at its single construction point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NominalDescriptor {
    /// A product type with named runtime fields.
    Struct {
        owner: FrozenTypeIdentity,
        fields: Vec<NominalFieldDescriptor>,
    },
    /// A sum type with named variants.
    Enum {
        owner: FrozenTypeIdentity,
        variants: Vec<NominalVariantDescriptor>,
    },
    /// A nominal wrapper over a single inner type.
    Newtype {
        owner: FrozenTypeIdentity,
        inner: FrozenTypeIdentity,
    },
    /// A semantically non-decomposable nominal (no visible representation).
    Opaque { owner: FrozenTypeIdentity },
}

impl NominalDescriptor {
    /// The sealed [`NominalShape`] discriminator this descriptor projects.
    pub(crate) fn shape(&self) -> NominalShape {
        match self {
            Self::Struct { .. } => NominalShape::Struct,
            Self::Enum { .. } => NominalShape::Enum,
            Self::Newtype { .. } => NominalShape::Newtype,
            Self::Opaque { .. } => NominalShape::Opaque,
        }
    }
}

/// ADR-009 B7 (Stage 2, Dec 50/94) — the ordered element identities of a
/// [`FrozenPayloadDescriptor::Tuple`]. Member order is significant; each entry
/// is the element type's frozen identity (position IS the index). Threaded from
/// the canonicalizer's `Tuple` arm so the payload reconstructs WITHOUT inverting
/// the one-way identity hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TupleDescriptor {
    pub(crate) elements: Vec<FrozenTypeIdentity>,
}

/// ADR-009 B7 — one normalized field of a [`FrozenPayloadDescriptor::Record`].
/// Carries the owner-bound HYGIENIC member identity (`#f`, minted from the
/// record's own identity + field name — NEVER the source-name string, Dec 57),
/// the field's VALUE-type frozen identity, and its `optional` flag.
///
/// ADR-009 E5 CKPT-3 (B2 in-scope, under the CKPT-0 record-hygiene binding
/// invariant): `name` is the field's plain source name, preserved ADDITIVELY as
/// a SPELL/REFLECT-ONLY freeze fact. It is populated in the SAME `canonical_record`
/// rebuild that mints the record identity + the hygienic `member` identity, but
/// is NEVER threaded into either computation — the record IDENTITY descriptor
/// string and `record_member_identity` stay byte-identical (the binding
/// invariant). It is identity-INSIGNIFICANT for hashing/dedup, yet already
/// identity-SIGNIFICANT for the record's own descriptor (which byte-sorts +
/// interpolates plain field names), so surfacing it adds no new information to
/// the identity algebra. Read ONLY at spell time (`reconstruct_type_annotation`'s
/// Record arm re-spells `{name: T}` from it) and reflect time
/// (`COMPTIME_RECORD_FIELD_SCHEMA.name`). Mirrors [`ParamDescriptor::name`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecordFieldDescriptor {
    pub(crate) member: FrozenTypeIdentity,
    pub(crate) type_identity: FrozenTypeIdentity,
    pub(crate) optional: bool,
    /// The field's plain source name — spell/reflect-only (see the type doc).
    pub(crate) name: String,
}

/// ADR-009 B7 — the normalized structural record of a
/// [`FrozenPayloadDescriptor::Record`]: its fields in member byte-sort order
/// (declaration-order independent; duplicate field names are a named rejection
/// at canonicalization, never a descriptor here).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecordDescriptor {
    pub(crate) fields: Vec<RecordFieldDescriptor>,
}

/// ADR-009 B7 — a reference type (`&T` / `&mut T`) of a
/// [`FrozenPayloadDescriptor::Reference`]: the mutability axis (descriptor-
/// significant) and the referent type's frozen identity. Shape has no declared
/// variance/ownership syntax on references, so `mutable` is the complete
/// representable axis (a bool, mirroring `RecordFieldDescriptor::optional` —
/// never a rendered `.kind` string).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReferenceDescriptor {
    pub(crate) mutable: bool,
    pub(crate) referent: FrozenTypeIdentity,
}

/// ADR-009 B7 — the set members of a [`FrozenPayloadDescriptor::Union`]:
/// deduped, byte-sorted member identities (a singleton union coalesces to its
/// member upstream, so a `Union` descriptor always carries ≥2 members).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UnionDescriptor {
    pub(crate) members: Vec<FrozenTypeIdentity>,
}

/// ADR-009 B7 Slice 2 — a declared trait bound of a
/// [`FrozenPayloadDescriptor::Parameter`].
///
/// Trait-reference bound descriptors are ticket B2 territory (the same
/// descriptors that inhabit [`FrozenErasedBound`]), so this element is
/// deliberately uninhabited: a non-empty parameter bound set is
/// unrepresentable today, which is the structural form of "no partial
/// descriptors" (spec §3.1) — never an empty (partial) bound set masquerading
/// as complete. Shape also has no declared variance/ownership syntax on type
/// parameters, so the type parameter's stable base-fn-scoped identity is the
/// COMPLETE representable payload today; B2 retypes this element (and the
/// overlay threads the declared bounds) without a parallel bound
/// representation that would collide with B2. Mirrors [`FrozenErasedBound`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrozenParameterBound {}

/// ADR-009 B7 Slice 2 (Stage 2, Dec 50/94) — the payload of
/// [`FrozenPayloadDescriptor::Parameter`]: a generic type parameter reflected
/// from inside its own generic body (the A3 base-fn-scoped-identity path).
///
/// `identity` is the parameter's STABLE `parameter:{owner}:{name}` frozen
/// identity — scoped to the BASE generic function name under monomorphization
/// (declaration-stable across instantiations, ADR-009 §Semantic Freeze
/// Decision 52), NEVER an analyzer inference variable and NEVER a mono key.
/// `bounds` is the declared trait bound set — provably empty today
/// ([`FrozenParameterBound`] is uninhabited until B2 lands the trait-reference
/// descriptors), the honest "bounds where representable" form. Deliberately NO
/// `Default`/partial constructor (rejection-matrix row R8) — every value is
/// fully populated at its single construction point
/// (`FreezeOverlay::payload_of`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TypeParamDescriptor {
    pub(crate) identity: FrozenTypeIdentity,
    pub(crate) bounds: Vec<FrozenParameterBound>,
}

/// Compiler-internal payload descriptor — the typed result of the semantic
/// freeze's payload query (the ONE query API, beside `identity_of` /
/// `category_of`). Covers exactly the enabled payload categories
/// (`FROZEN_TYPE_ENABLED_PAYLOAD_CATEGORIES`: Primitive / Never / Erased);
/// a non-enabled category is a named R1 rejection at the query, never a
/// variant here. Deliberately NO `Default` and no empty/partial constructor
/// (rejection-matrix row R8) — every value is fully populated at its single
/// construction point (`FrozenTypeIndex::payload_for_identity`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum FrozenPayloadDescriptor {
    /// The sealed `FrozenPrimitive` sub-algebra member with exact
    /// width/domain payload (Dec 50/94).
    Primitive(FrozenPrimitive),
    /// The uninhabited type.
    Never,
    /// Erased type with its bound set — reachable today solely as `any`
    /// (the empty set; see [`FrozenErasedBound`]).
    Erased { bounds: Vec<FrozenErasedBound> },
    /// ADR-009 B6 (Dec 63): a fully-inferred callable signature descriptor —
    /// ordered positional parameters (stable identity, type, optionality,
    /// passing mode) and the return type identity. Issued only AFTER the
    /// signature freezes (Dec 52: the freeze-boundary predicate rejects
    /// issuance for an unresolved signature before any hook runs).
    Callable(CallableDescriptor),
    /// ADR-009 B5 (Dec 55): a nominal declaration-shape descriptor —
    /// Struct / Enum / Newtype / Opaque. Issued only for a RESOLVED nominal
    /// (an un-applied generic head or an unsubstituted applied form is a named
    /// rejection at the query, never a descriptor here).
    Nominal(NominalDescriptor),
    /// ADR-009 B7 (Dec 50/94): a tuple type's ordered element identities.
    Tuple(TupleDescriptor),
    /// ADR-009 B7 (Dec 50/94): a normalized structural record's fields.
    Record(RecordDescriptor),
    /// ADR-009 B7 (Dec 50/94): a reference type (`&T` / `&mut T`).
    Reference(ReferenceDescriptor),
    /// ADR-009 B7 (Dec 50/94): a normalized union's member identities.
    Union(UnionDescriptor),
    /// ADR-009 B7 Slice 2 (Dec 50/94): a generic type parameter reflected from
    /// inside its own generic body — its stable base-fn-scoped identity + the
    /// (provably-empty until B2) declared bound set. Issued only off a scoped
    /// `parameter:{owner}:{name}` overlay identity (the A3 path), never an
    /// inference hole.
    Parameter(TypeParamDescriptor),
}

impl FrozenPayloadDescriptor {
    /// The catalog category this payload refines (`FrozenTypeCategory` stays
    /// the exhaustive 10-variant layer; payloads never fork the catalog).
    pub(crate) fn category(&self) -> FrozenTypeCategory {
        match self {
            Self::Primitive(_) => FrozenTypeCategory::Primitive,
            Self::Never => FrozenTypeCategory::Never,
            Self::Erased { .. } => FrozenTypeCategory::Erased,
            Self::Callable(_) => FrozenTypeCategory::Callable,
            Self::Nominal(_) => FrozenTypeCategory::Nominal,
            Self::Tuple(_) => FrozenTypeCategory::Tuple,
            Self::Record(_) => FrozenTypeCategory::Record,
            Self::Reference(_) => FrozenTypeCategory::Reference,
            Self::Union(_) => FrozenTypeCategory::Union,
            Self::Parameter(_) => FrozenTypeCategory::Parameter,
        }
    }
}

/// Rejection-matrix row R1 (sanctioned tracer pattern): ONE named
/// compile-time diagnostic per non-enabled category — naming the category,
/// stating its payload descriptor has not landed, and pointing at the
/// exhaustive `type_category` — never a partial descriptor.
pub(crate) fn pending_payload_rejection(category: FrozenTypeCategory) -> String {
    format!(
        "reflect: the {} payload descriptor has not landed (pending payload \
         ticket); use type_category for the exhaustive category",
        category.variant_name()
    )
}

/// The A2×B1 seam's Erased disposition (sanctioned tracer pattern, same R1
/// family as [`pending_payload_rejection`]): `dyn Trait` /
/// trait-intersection spellings classify as the ENABLED Erased category,
/// but their bound-set payload elements are the trait-reference descriptors
/// that land with ticket B2 ([`FrozenErasedBound`] is deliberately
/// uninhabited until then). Reflecting a BOUNDED erased identity is
/// therefore this named rejection — never an empty (partial) bound set.
/// Only the base-frozen `any` leaf answers the Erased payload today.
/// ADR-009 B5 rejection (drift note R10/R11 family): a BARE generic
/// constructor head (`Array`, `Option`, `HashMap`, a user generic) is
/// `TypeConstructorRef` territory (ticket B4), NOT a resolved nominal shape.
/// Reflecting an un-applied head is this named rejection — never a
/// `StructDescriptor` issued off the un-applied form.
pub(crate) fn unapplied_generic_head_rejection() -> String {
    "reflect: an un-applied generic type constructor is not a resolved nominal shape; \
     apply its type arguments (type_constructor(Head).apply(...)) or reflect an applied \
     form before reflecting its shape"
        .to_string()
}

/// ADR-009 B5 rejection (Dec 55, R10): reflecting an APPLIED generic nominal
/// (`Option<int>`, `Page<User>`) requires generic substitution before
/// descriptor issuance — the applied identity's substituted member types
/// (threaded through the B4 applied identities). Until that slice lands it is
/// this named pending rejection, never a descriptor issued off the
/// un-substituted form.
pub(crate) fn applied_nominal_pending_rejection() -> String {
    "reflect: reflecting an applied generic nominal requires generic substitution before \
     descriptor issuance (pending slice); reflect the base nominal, or use type_category \
     for the exhaustive category"
        .to_string()
}

pub(crate) fn bounded_erased_payload_rejection() -> String {
    "reflect: the Erased bound-set payload for trait-object bounds has not \
     landed (pending ticket B2 trait-reference descriptors); use \
     type_category for the exhaustive category"
        .to_string()
}

/// Build the `FrozenType` comptime value for a frozen identity: the sealed
/// sum's enum object (variant id = Dec 50/94 catalog ordinal) wrapping the
/// nested payload descriptor object. Mirrors
/// `build_frozen_type_category_heap_value`; R1/unknown-identity rejections
/// propagate from the freeze's payload query.
pub(crate) fn build_frozen_type_heap_value(
    identity: FrozenTypeIdentity,
    freeze: &FreezeOverlay,
) -> Result<HeapValue, String> {
    let payload = freeze.payload_of(identity)?;
    let category = payload.category();
    let payload_slot = match payload {
        FrozenPayloadDescriptor::Primitive(primitive) => {
            frozen_primitive_descriptor_slot(primitive)?
        }
        FrozenPayloadDescriptor::Never => {
            typed_object_for_named_schema(COMPTIME_FROZEN_NEVER_SCHEMA, &[])
        }
        FrozenPayloadDescriptor::Erased { bounds } => frozen_erased_descriptor_slot(&bounds),
        FrozenPayloadDescriptor::Callable(descriptor) => {
            frozen_callable_descriptor_slot(&descriptor)?
        }
        FrozenPayloadDescriptor::Nominal(descriptor) => {
            frozen_nominal_descriptor_slot(&descriptor)?
        }
        FrozenPayloadDescriptor::Tuple(descriptor) => frozen_tuple_descriptor_slot(&descriptor)?,
        FrozenPayloadDescriptor::Record(descriptor) => frozen_record_descriptor_slot(&descriptor)?,
        FrozenPayloadDescriptor::Reference(descriptor) => {
            frozen_reference_descriptor_slot(&descriptor)
        }
        FrozenPayloadDescriptor::Union(descriptor) => frozen_union_descriptor_slot(&descriptor)?,
        FrozenPayloadDescriptor::Parameter(descriptor) => {
            frozen_parameter_descriptor_slot(&descriptor)
        }
    };
    let variant = enum_variant_id(COMPTIME_FROZEN_TYPE_SCHEMA, category.variant_name())?;
    super::typed_slot_into_heap_value(typed_object_for_named_schema(
        COMPTIME_FROZEN_TYPE_SCHEMA,
        &[
            ("__variant", KindedSlot::from_int(variant)),
            ("__payload_0", payload_slot),
        ],
    ))
}

/// The nested `FrozenPrimitive` descriptor object: variant id from the
/// catalog-generated schema; integer/float family variants carry their
/// width-domain enum object, scalar members carry the Null payload slot.
fn frozen_primitive_descriptor_slot(primitive: FrozenPrimitive) -> Result<KindedSlot, String> {
    let width_slot = match primitive {
        FrozenPrimitive::SignedInteger(width) | FrozenPrimitive::UnsignedInteger(width) => {
            unit_enum_variant_slot(INTEGER_WIDTH_SCHEMA_NAME, width.variant_name())?
        }
        FrozenPrimitive::BinaryFloat(width) => {
            unit_enum_variant_slot(FLOAT_WIDTH_SCHEMA_NAME, width.variant_name())?
        }
        FrozenPrimitive::Unit
        | FrozenPrimitive::Bool
        | FrozenPrimitive::Char
        | FrozenPrimitive::Decimal
        | FrozenPrimitive::String
        | FrozenPrimitive::Null
        | FrozenPrimitive::Undefined => KindedSlot::none(),
    };
    let variant = enum_variant_id(COMPTIME_FROZEN_PRIMITIVE_SCHEMA, primitive.variant_name())?;
    Ok(typed_object_for_named_schema(
        COMPTIME_FROZEN_PRIMITIVE_SCHEMA,
        &[
            ("__variant", KindedSlot::from_int(variant)),
            ("__payload_0", width_slot),
        ],
    ))
}

/// The nested `FrozenErased` descriptor object carrying the bound-set
/// array. [`FrozenErasedBound`] is uninhabited until B2, so the array is
/// provably empty — the exhaustive match below is the structural proof, not
/// an assumption.
fn frozen_erased_descriptor_slot(bounds: &[FrozenErasedBound]) -> KindedSlot {
    for bound in bounds {
        match *bound {}
    }
    let bounds_array = TypedArray::<*const TypedObjectStorage>::with_capacity(0);
    // SAFETY: freshly allocated array pointer; stamping the element type is
    // the same construction pattern as `comptime_target`'s object arrays.
    unsafe {
        stamp_elem_type(bounds_array as *mut u8, ELEM_TYPE_TYPED_OBJECT);
    }
    typed_object_for_named_schema(
        COMPTIME_FROZEN_ERASED_SCHEMA,
        &[(
            "bounds",
            KindedSlot::new(
                ValueSlot::from_raw(bounds_array as usize as u64),
                NativeKind::Ptr(HeapKind::TypedArray),
            ),
        )],
    )
}

/// ADR-009 B6: build the `FrozenCallable` descriptor object — the ordered
/// `params` array (each a `ParamDescriptor` object) plus the return type's
/// frozen identity halves. Typed descriptor data all the way down: identities
/// and nested typed objects, never rendered type-name strings.
fn frozen_callable_descriptor_slot(descriptor: &CallableDescriptor) -> Result<KindedSlot, String> {
    let mut param_objs: Vec<KindedSlot> = Vec::with_capacity(descriptor.params.len());
    for param in &descriptor.params {
        param_objs.push(param_descriptor_slot(param)?);
    }
    let params_array = object_array_slot(param_objs)?;
    Ok(typed_object_for_named_schema(
        COMPTIME_FROZEN_CALLABLE_SCHEMA,
        &[
            ("params", params_array),
            (
                "returns_identity_high",
                KindedSlot::from_int(descriptor.returns.high),
            ),
            (
                "returns_identity_low",
                KindedSlot::from_int(descriptor.returns.low),
            ),
        ],
    ))
}

/// ADR-009 B6: build one `ParamDescriptor` object — the parameter's value-type
/// frozen identity halves, its `optional` flag, and its `PassingMode` enum
/// carrier (`Move` / `SharedBorrow` / `ExclusiveBorrow`). Parameter names are
/// identity-insignificant and stay a freeze fact — never a runtime string.
fn param_descriptor_slot(param: &ParamDescriptor) -> Result<KindedSlot, String> {
    let mode_slot = unit_enum_variant_slot(PASSING_MODE_SCHEMA_NAME, param.mode.variant_name())?;
    Ok(typed_object_for_named_schema(
        COMPTIME_FROZEN_PARAM_DESCRIPTOR_SCHEMA,
        &[
            (
                "type_identity_high",
                KindedSlot::from_int(param.type_identity.high),
            ),
            (
                "type_identity_low",
                KindedSlot::from_int(param.type_identity.low),
            ),
            ("optional", KindedSlot::from_bool(param.optional)),
            ("mode", mode_slot),
        ],
    ))
}

/// ADR-009 B5: build the `FrozenNominal` descriptor object — the sealed
/// `NominalShape` enum carrier wrapping the shape's typed row descriptor. Typed
/// descriptor data all the way down: identities and nested typed objects, never
/// rendered type-name strings, never a `.kind` string (Dec 55 rejection R4).
fn frozen_nominal_descriptor_slot(descriptor: &NominalDescriptor) -> Result<KindedSlot, String> {
    let shape_payload = match descriptor {
        NominalDescriptor::Struct { owner, fields } => struct_descriptor_slot(*owner, fields)?,
        NominalDescriptor::Enum { owner, variants } => enum_descriptor_slot(*owner, variants)?,
        NominalDescriptor::Newtype { owner, inner } => newtype_descriptor_slot(*owner, *inner),
        NominalDescriptor::Opaque { owner } => opaque_descriptor_slot(*owner),
    };
    let shape_slot = nominal_shape_enum_slot(descriptor.shape(), shape_payload)?;
    Ok(typed_object_for_named_schema(
        COMPTIME_FROZEN_NOMINAL_SCHEMA,
        &[("shape", shape_slot)],
    ))
}

/// The `NominalShape` enum value: variant id from the catalog-generated schema
/// (Dec 55), `__payload_0` = the shape's typed row descriptor object.
fn nominal_shape_enum_slot(shape: NominalShape, payload: KindedSlot) -> Result<KindedSlot, String> {
    let variant = enum_variant_id(NOMINAL_SHAPE_SCHEMA_NAME, shape.variant_name())?;
    Ok(typed_object_for_named_schema(
        NOMINAL_SHAPE_SCHEMA_NAME,
        &[
            ("__variant", KindedSlot::from_int(variant)),
            ("__payload_0", payload),
        ],
    ))
}

/// The `StructDescriptor` object: owner identity halves, the runtime field
/// count, and the ordered typed `FieldDescriptor` array.
fn struct_descriptor_slot(
    owner: FrozenTypeIdentity,
    fields: &[NominalFieldDescriptor],
) -> Result<KindedSlot, String> {
    let mut field_objs = Vec::with_capacity(fields.len());
    for field in fields {
        field_objs.push(field_descriptor_slot(owner, field)?);
    }
    let fields_array = object_array_slot(field_objs)?;
    Ok(typed_object_for_named_schema(
        COMPTIME_STRUCT_DESCRIPTOR_SCHEMA,
        &[
            ("owner_identity_high", KindedSlot::from_int(owner.high)),
            ("owner_identity_low", KindedSlot::from_int(owner.low)),
            ("field_count", KindedSlot::from_int(fields.len() as i64)),
            ("fields", fields_array),
        ],
    ))
}

/// One `FieldDescriptor` object: owner + owner-bound member identity + value
/// type identity + the `FieldInitialization` enum carrier. No source-name
/// string (Dec 57).
fn field_descriptor_slot(
    owner: FrozenTypeIdentity,
    field: &NominalFieldDescriptor,
) -> Result<KindedSlot, String> {
    let init_slot = unit_enum_variant_slot(
        FIELD_INITIALIZATION_SCHEMA_NAME,
        field.initialization.variant_name(),
    )?;
    Ok(typed_object_for_named_schema(
        COMPTIME_FIELD_DESCRIPTOR_SCHEMA,
        &[
            ("owner_identity_high", KindedSlot::from_int(owner.high)),
            ("owner_identity_low", KindedSlot::from_int(owner.low)),
            ("member_high", KindedSlot::from_int(field.member.high)),
            ("member_low", KindedSlot::from_int(field.member.low)),
            (
                "type_identity_high",
                KindedSlot::from_int(field.type_identity.high),
            ),
            (
                "type_identity_low",
                KindedSlot::from_int(field.type_identity.low),
            ),
            ("initialization", init_slot),
        ],
    ))
}

/// The `EnumDescriptor` object: owner identity halves, the variant count, and
/// the ordered typed `VariantDescriptor` array.
fn enum_descriptor_slot(
    owner: FrozenTypeIdentity,
    variants: &[NominalVariantDescriptor],
) -> Result<KindedSlot, String> {
    let mut variant_objs = Vec::with_capacity(variants.len());
    for variant in variants {
        variant_objs.push(variant_descriptor_slot(owner, variant)?);
    }
    let variants_array = object_array_slot(variant_objs)?;
    Ok(typed_object_for_named_schema(
        COMPTIME_ENUM_DESCRIPTOR_SCHEMA,
        &[
            ("owner_identity_high", KindedSlot::from_int(owner.high)),
            ("owner_identity_low", KindedSlot::from_int(owner.low)),
            ("variant_count", KindedSlot::from_int(variants.len() as i64)),
            ("variants", variants_array),
        ],
    ))
}

/// One `VariantDescriptor` object: owner + owner-bound member identity + the
/// payload arity.
fn variant_descriptor_slot(
    owner: FrozenTypeIdentity,
    variant: &NominalVariantDescriptor,
) -> Result<KindedSlot, String> {
    Ok(typed_object_for_named_schema(
        COMPTIME_VARIANT_DESCRIPTOR_SCHEMA,
        &[
            ("owner_identity_high", KindedSlot::from_int(owner.high)),
            ("owner_identity_low", KindedSlot::from_int(owner.low)),
            ("member_high", KindedSlot::from_int(variant.member.high)),
            ("member_low", KindedSlot::from_int(variant.member.low)),
            (
                "payload_arity",
                KindedSlot::from_int(i64::from(variant.payload_arity)),
            ),
        ],
    ))
}

/// The `NewtypeDescriptor` object: owner identity halves + the single inner
/// type's frozen identity halves.
fn newtype_descriptor_slot(owner: FrozenTypeIdentity, inner: FrozenTypeIdentity) -> KindedSlot {
    typed_object_for_named_schema(
        COMPTIME_NEWTYPE_DESCRIPTOR_SCHEMA,
        &[
            ("owner_identity_high", KindedSlot::from_int(owner.high)),
            ("owner_identity_low", KindedSlot::from_int(owner.low)),
            ("inner_identity_high", KindedSlot::from_int(inner.high)),
            ("inner_identity_low", KindedSlot::from_int(inner.low)),
        ],
    )
}

/// The `OpaqueTypeDescriptor` object: owner identity halves only.
fn opaque_descriptor_slot(owner: FrozenTypeIdentity) -> KindedSlot {
    typed_object_for_named_schema(
        COMPTIME_OPAQUE_TYPE_DESCRIPTOR_SCHEMA,
        &[
            ("owner_identity_high", KindedSlot::from_int(owner.high)),
            ("owner_identity_low", KindedSlot::from_int(owner.low)),
        ],
    )
}

/// ADR-009 B7: build the `FrozenTuple` descriptor object — the ordered
/// `elements` array (each a `TupleElement` object: positional index + the
/// element type's frozen identity halves). Typed descriptor data all the way
/// down: identities and nested typed objects, never rendered type-name strings.
fn frozen_tuple_descriptor_slot(descriptor: &TupleDescriptor) -> Result<KindedSlot, String> {
    let mut element_objs = Vec::with_capacity(descriptor.elements.len());
    for (index, element) in descriptor.elements.iter().enumerate() {
        element_objs.push(tuple_element_slot(index, *element));
    }
    let elements_array = object_array_slot(element_objs)?;
    Ok(typed_object_for_named_schema(
        COMPTIME_FROZEN_TUPLE_SCHEMA,
        &[("elements", elements_array)],
    ))
}

/// One `TupleElement` object: the positional index + the element type's frozen
/// identity halves.
fn tuple_element_slot(index: usize, element: FrozenTypeIdentity) -> KindedSlot {
    typed_object_for_named_schema(
        COMPTIME_TUPLE_ELEMENT_SCHEMA,
        &[
            ("index", KindedSlot::from_int(index as i64)),
            ("type_identity_high", KindedSlot::from_int(element.high)),
            ("type_identity_low", KindedSlot::from_int(element.low)),
        ],
    )
}

/// ADR-009 B7: build the `FrozenRecord` descriptor object — the normalized
/// `fields` array (each a `RecordField` object: owner-bound hygienic member
/// identity + value-type identity + optionality). No source-name strings
/// (Dec 57).
fn frozen_record_descriptor_slot(descriptor: &RecordDescriptor) -> Result<KindedSlot, String> {
    let mut field_objs = Vec::with_capacity(descriptor.fields.len());
    for field in &descriptor.fields {
        field_objs.push(record_field_slot(field));
    }
    let fields_array = object_array_slot(field_objs)?;
    Ok(typed_object_for_named_schema(
        COMPTIME_FROZEN_RECORD_SCHEMA,
        &[("fields", fields_array)],
    ))
}

/// One `RecordField` object: owner-bound hygienic member identity halves + the
/// field's value-type frozen identity halves + the `optional` flag + the plain
/// field `name`. ADR-009 E5 CKPT-3: `name` is the spell/reflect-only freeze fact
/// (a `String`-kind slot, refcount-cloned into the object by the schema builder —
/// same pattern as `nb_str` populating `__ComptimeTypeInfo.name`); the member
/// identity remains the identity/dedup-bearing handle (Dec 57).
fn record_field_slot(field: &RecordFieldDescriptor) -> KindedSlot {
    typed_object_for_named_schema(
        COMPTIME_RECORD_FIELD_SCHEMA,
        &[
            ("member_high", KindedSlot::from_int(field.member.high)),
            ("member_low", KindedSlot::from_int(field.member.low)),
            (
                "type_identity_high",
                KindedSlot::from_int(field.type_identity.high),
            ),
            (
                "type_identity_low",
                KindedSlot::from_int(field.type_identity.low),
            ),
            ("optional", KindedSlot::from_bool(field.optional)),
            ("name", KindedSlot::from_string(&field.name)),
        ],
    )
}

/// ADR-009 B7: build the `FrozenReference` descriptor object — the `mutable`
/// flag (`&T` vs `&mut T`) + the referent type's frozen identity halves.
fn frozen_reference_descriptor_slot(descriptor: &ReferenceDescriptor) -> KindedSlot {
    typed_object_for_named_schema(
        COMPTIME_FROZEN_REFERENCE_SCHEMA,
        &[
            ("mutable", KindedSlot::from_bool(descriptor.mutable)),
            (
                "referent_identity_high",
                KindedSlot::from_int(descriptor.referent.high),
            ),
            (
                "referent_identity_low",
                KindedSlot::from_int(descriptor.referent.low),
            ),
        ],
    )
}

/// ADR-009 B7: build the `FrozenUnion` descriptor object — the set `members`
/// array (each a `UnionMember` object: the member type's frozen identity
/// halves), in the deduped byte-sorted order the canonical descriptor fixes.
fn frozen_union_descriptor_slot(descriptor: &UnionDescriptor) -> Result<KindedSlot, String> {
    let mut member_objs = Vec::with_capacity(descriptor.members.len());
    for member in &descriptor.members {
        member_objs.push(union_member_slot(*member));
    }
    let members_array = object_array_slot(member_objs)?;
    Ok(typed_object_for_named_schema(
        COMPTIME_FROZEN_UNION_SCHEMA,
        &[("members", members_array)],
    ))
}

/// One `UnionMember` object: the member type's frozen identity halves.
fn union_member_slot(member: FrozenTypeIdentity) -> KindedSlot {
    typed_object_for_named_schema(
        COMPTIME_UNION_MEMBER_SCHEMA,
        &[
            ("type_identity_high", KindedSlot::from_int(member.high)),
            ("type_identity_low", KindedSlot::from_int(member.low)),
        ],
    )
}

/// ADR-009 B7 Slice 2: build the `FrozenParameter` descriptor object — the type
/// parameter's stable base-fn-scoped frozen identity halves + the bound-set
/// array. [`FrozenParameterBound`] is uninhabited until B2, so the array is
/// provably empty — the exhaustive match below is the structural proof, not an
/// assumption (mirrors `frozen_erased_descriptor_slot`). Typed descriptor data
/// only: identity halves + an empty typed array, never a rendered type-name
/// string.
fn frozen_parameter_descriptor_slot(descriptor: &TypeParamDescriptor) -> KindedSlot {
    for bound in &descriptor.bounds {
        match *bound {}
    }
    let bounds_array = TypedArray::<*const TypedObjectStorage>::with_capacity(0);
    // SAFETY: freshly allocated array pointer; stamping the element type mirrors
    // the sanctioned `frozen_erased_descriptor_slot` empty-array construction.
    unsafe {
        stamp_elem_type(bounds_array as *mut u8, ELEM_TYPE_TYPED_OBJECT);
    }
    typed_object_for_named_schema(
        COMPTIME_FROZEN_PARAMETER_SCHEMA,
        &[
            (
                "identity_high",
                KindedSlot::from_int(descriptor.identity.high),
            ),
            (
                "identity_low",
                KindedSlot::from_int(descriptor.identity.low),
            ),
            (
                "bounds",
                KindedSlot::new(
                    ValueSlot::from_raw(bounds_array as usize as u64),
                    NativeKind::Ptr(HeapKind::TypedArray),
                ),
            ),
        ],
    )
}

/// Build an `Array<TypedObject>` slot carried by a stamped v2-raw
/// `TypedArray<*const TypedObjectStorage>`. Each element's refcount share is
/// transferred into the array (mirrors `comptime_target::nb_object_array`,
/// the sanctioned object-array construction pattern).
fn object_array_slot(objs: Vec<KindedSlot>) -> Result<KindedSlot, String> {
    let arr = TypedArray::<*const TypedObjectStorage>::with_capacity(objs.len() as u32);
    // SAFETY: freshly allocated array pointer; stamping the element type +
    // pushing owned `*const TypedObjectStorage` shares mirrors the sanctioned
    // `nb_object_array` construction pattern.
    unsafe {
        stamp_elem_type(arr as *mut u8, ELEM_TYPE_TYPED_OBJECT);
    }
    for obj in objs {
        if obj.kind() != NativeKind::Ptr(HeapKind::TypedObject) {
            unsafe {
                TypedArray::<*const TypedObjectStorage>::drop_array_heap(arr);
            }
            return Err(format!(
                "comptime descriptor object array expected a TypedObject element, got {:?}",
                obj.kind()
            ));
        }
        let ptr = obj.raw() as *const TypedObjectStorage;
        unsafe {
            TypedArray::<*const TypedObjectStorage>::push(arr, ptr);
        }
        // Transfer the element's refcount share into the array.
        std::mem::forget(obj);
    }
    Ok(KindedSlot::new(
        ValueSlot::from_raw(arr as usize as u64),
        NativeKind::Ptr(HeapKind::TypedArray),
    ))
}

/// A unit-variant value of a catalog-generated enum schema
/// (`IntegerWidth` / `FloatWidth`).
fn unit_enum_variant_slot(schema_name: &str, variant_name: &str) -> Result<KindedSlot, String> {
    let variant = enum_variant_id(schema_name, variant_name)?;
    Ok(typed_object_for_named_schema(
        schema_name,
        &[("__variant", KindedSlot::from_int(variant))],
    ))
}

/// Resolve a variant id by name in the ambient registry's enum schema.
fn enum_variant_id(schema_name: &str, variant_name: &str) -> Result<i64, String> {
    let registry = current_registry();
    let schema = registry
        .get(schema_name)
        .ok_or_else(|| format!("descriptor schema {schema_name:?} is not registered"))?;
    schema
        .variant_id(variant_name)
        .map(i64::from)
        .ok_or_else(|| format!("descriptor schema {schema_name:?} has no '{variant_name}' variant"))
}
