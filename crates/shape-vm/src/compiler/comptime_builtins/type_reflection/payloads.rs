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
    FLOAT_WIDTH_SCHEMA_NAME, FrozenPrimitive, INTEGER_WIDTH_SCHEMA_NAME,
};
use shape_runtime::type_schema::builtin_schemas::{
    COMPTIME_FROZEN_ERASED_SCHEMA, COMPTIME_FROZEN_NEVER_SCHEMA, COMPTIME_FROZEN_PRIMITIVE_SCHEMA,
    COMPTIME_FROZEN_TYPE_SCHEMA,
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
}

impl FrozenPayloadDescriptor {
    /// The catalog category this payload refines (`FrozenTypeCategory` stays
    /// the exhaustive 10-variant layer; payloads never fork the catalog).
    pub(crate) fn category(&self) -> FrozenTypeCategory {
        match self {
            Self::Primitive(_) => FrozenTypeCategory::Primitive,
            Self::Never => FrozenTypeCategory::Never,
            Self::Erased { .. } => FrozenTypeCategory::Erased,
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
