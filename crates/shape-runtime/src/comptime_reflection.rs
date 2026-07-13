use crate::builtin_metadata::{BuiltinMetadata, BuiltinParam};
use crate::type_schema::builtin_schemas::{
    COMPTIME_FROZEN_ERASED_SCHEMA, COMPTIME_FROZEN_NEVER_SCHEMA,
    COMPTIME_FROZEN_PRIMITIVE_SCHEMA, COMPTIME_FROZEN_TYPE_REF_SCHEMA,
    COMPTIME_FROZEN_TYPE_SCHEMA,
};
use shape_value::heap_value::HeapKind;
use shape_value::{KindedSlot, NativeKind};

/// ADR-009 A1 S4 — single-source catalog macro.
///
/// One variant list generates the `FrozenTypeCategory` enum, its exhaustive
/// `ALL` table, `variant_name`, and the human-readable category enumeration
/// embedded in the LSP-visible `type_category` builtin row. Because every
/// artifact expands from the same token list, the metadata row cannot drift
/// from the semantic catalog the compiler and LSP completion query
/// (`FrozenTypeCategory::ALL`). There is deliberately no second, hand-written
/// variant list anywhere.
macro_rules! frozen_type_category_catalog {
    ($first:ident $(, $rest:ident)* $(,)?) => {
        /// Exhaustive top-level semantic categories exposed by typed comptime
        /// reflection. Payload-bearing compiler descriptors refine these
        /// categories; there is deliberately no unknown or inference-variable
        /// arm.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum FrozenTypeCategory {
            $first
            $(, $rest)*
        }

        impl FrozenTypeCategory {
            pub const ALL: [Self; [stringify!($first) $(, stringify!($rest))*].len()] =
                [Self::$first $(, Self::$rest)*];

            pub const fn variant_name(self) -> &'static str {
                match self {
                    Self::$first => stringify!($first),
                    $(Self::$rest => stringify!($rest),)*
                }
            }

            /// The category's declaration ordinal in the Dec 50/94 catalog
            /// (ADR-009 B1 S1). Payload-variant ids in the `FrozenType`
            /// descriptor schema are pinned to THESE ordinals — never
            /// densely renumbered — so later B tickets add payload variants
            /// without changing existing ids (comptime-ABI stability, spec
            /// §3.3).
            pub const fn catalog_ordinal(self) -> u16 {
                self as u16
            }
        }

        /// Compile-time rendering of the exhaustive category catalog
        /// (`` `Variant` | `Variant` | … ``), generated from the same variant
        /// list as [`FrozenTypeCategory`] itself.
        pub const FROZEN_TYPE_CATEGORY_VARIANTS_DOC: &str =
            concat!("`", stringify!($first), "`" $(, " | `", stringify!($rest), "`")*);

        /// Description text for the LSP-visible `type_category` builtin row.
        /// The category enumeration is generated from the catalog variant
        /// list — never hand-written.
        const TYPE_CATEGORY_ROW_DESCRIPTION: &str = concat!(
            "Return the exhaustive semantic category of an opaque TypeRef. \
             The result is exactly one of ",
            "`", stringify!($first), "`" $(, " | `", stringify!($rest), "`")*,
            ". Only valid inside comptime blocks."
        );
    };
}

frozen_type_category_catalog!(
    Primitive, Never, Parameter, Nominal, Tuple, Record, Callable, Reference, Union, Erased,
);

/// Descriptor row for one `FrozenPrimitive` catalog member: the variant name
/// and its typed payload arity (1 for the width/domain-carrying family
/// variants, 0 for scalar members). Drives the descriptor-schema
/// registration and the catalog-completeness tests — the same table the
/// enum expands from, so no second variant list can drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrozenPrimitiveVariantDescriptor {
    pub name: &'static str,
    pub payload_arity: u16,
    /// ADR-009 B1 S3: the typed width/domain payload TYPE name for the
    /// arity-1 family variants (`IntegerWidth` / `FloatWidth`), generated
    /// from the same macro token list as the enum payloads themselves.
    /// Consumed by the mini-VM payload-model item injection — no second
    /// hand-written variant/payload list can drift.
    pub payload_type: Option<&'static str>,
}

/// Width domain for the signed/unsigned integer families of the sealed
/// `FrozenPrimitive` sub-algebra (Dec 50/94: "width and other semantic
/// parameters are typed descriptor data, not rendered names").
///
/// `Arbitrary` is the explicit unbounded-width member: `bigint` is a signed
/// integer-family member with `Arbitrary` width — a NAMED design decision
/// (ADR-009 B1 S1), not an "unresolved"-style bucket. The rejected
/// alternative (a dedicated `BigInt` catalog variant) is logged in
/// `docs/defections.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IntegerWidth {
    W8,
    W16,
    W32,
    W64,
    Arbitrary,
}

impl IntegerWidth {
    pub const ALL: [Self; 5] = [Self::W8, Self::W16, Self::W32, Self::W64, Self::Arbitrary];

    /// Catalog variant name (drives the `IntegerWidth` descriptor-schema
    /// registration; exhaustive so a new domain member forces an update).
    pub const fn variant_name(self) -> &'static str {
        match self {
            Self::W8 => "W8",
            Self::W16 => "W16",
            Self::W32 => "W32",
            Self::W64 => "W64",
            Self::Arbitrary => "Arbitrary",
        }
    }
}

/// Schema/type name for the [`IntegerWidth`] width-domain enum carrier
/// (ADR-009 B1 S2). Spellable, following the `FrozenTypeCategory`
/// precedent — user comptime code matches its variants; the
/// [`runtime_lift_rejection`] arm keeps the values out of runtime code.
pub const INTEGER_WIDTH_SCHEMA_NAME: &str = "IntegerWidth";

/// Width domain for the binary floating-point family of the sealed
/// `FrozenPrimitive` sub-algebra (`f32` / `f64`-`number`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FloatWidth {
    W32,
    W64,
}

impl FloatWidth {
    pub const ALL: [Self; 2] = [Self::W32, Self::W64];

    /// Catalog variant name (drives the `FloatWidth` descriptor-schema
    /// registration; exhaustive so a new domain member forces an update).
    pub const fn variant_name(self) -> &'static str {
        match self {
            Self::W32 => "W32",
            Self::W64 => "W64",
        }
    }
}

/// Schema/type name for the [`FloatWidth`] width-domain enum carrier
/// (ADR-009 B1 S2). Same discipline as [`INTEGER_WIDTH_SCHEMA_NAME`].
pub const FLOAT_WIDTH_SCHEMA_NAME: &str = "FloatWidth";

/// ADR-009 B1 S1 — single-source catalog macro for the sealed
/// `FrozenPrimitive` sub-algebra (Dec 50/94), sibling of
/// [`frozen_type_category_catalog!`].
///
/// One token list generates the `FrozenPrimitive` enum (with typed
/// width/domain payloads on the family variants), the
/// `FROZEN_PRIMITIVE_VARIANTS` descriptor table that the schema registration
/// consumes, `variant_name`, and the human-readable variant enumeration.
/// Because every artifact expands from the same token list, the descriptor
/// schema cannot drift from the semantic catalog. There is deliberately no
/// unknown or inference-variable arm, and no `Default`/empty constructor.
macro_rules! frozen_primitive_catalog {
    (@arity) => {
        0u16
    };
    (@arity $payload:ident) => {
        1u16
    };
    (@payload_type) => {
        None
    };
    (@payload_type $payload:ident) => {
        Some(stringify!($payload))
    };
    (
        $first:ident $(($first_payload:ident))?
        $(, $rest:ident $(($rest_payload:ident))? )* $(,)?
    ) => {
        /// The sealed, exhaustive `FrozenPrimitive` sub-algebra (Dec 50/94):
        /// unit, bool, char, the signed/unsigned integer families, the
        /// binary float family, exact decimal, string, null, undefined.
        /// Width/domain data is typed payload on the family variants —
        /// never rendered names. There is deliberately no unknown arm.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum FrozenPrimitive {
            $first $(($first_payload))?
            $(, $rest $(($rest_payload))? )*
        }

        impl FrozenPrimitive {
            pub const fn variant_name(&self) -> &'static str {
                match self {
                    Self::$first { .. } => stringify!($first),
                    $(Self::$rest { .. } => stringify!($rest),)*
                }
            }
        }

        /// Descriptor table generated from the same token list as
        /// [`FrozenPrimitive`] itself — the single source the
        /// `FrozenPrimitive` descriptor-schema registration and the
        /// completeness tests consume.
        pub const FROZEN_PRIMITIVE_VARIANTS:
            [FrozenPrimitiveVariantDescriptor;
                [stringify!($first) $(, stringify!($rest))*].len()] = [
            FrozenPrimitiveVariantDescriptor {
                name: stringify!($first),
                payload_arity: frozen_primitive_catalog!(@arity $($first_payload)?),
                payload_type: frozen_primitive_catalog!(@payload_type $($first_payload)?),
            }
            $(, FrozenPrimitiveVariantDescriptor {
                name: stringify!($rest),
                payload_arity: frozen_primitive_catalog!(@arity $($rest_payload)?),
                payload_type: frozen_primitive_catalog!(@payload_type $($rest_payload)?),
            })*
        ];

        /// Compile-time rendering of the exhaustive `FrozenPrimitive`
        /// catalog, generated from the same variant list as the enum.
        pub const FROZEN_PRIMITIVE_VARIANTS_DOC: &str =
            concat!("`", stringify!($first), "`" $(, " | `", stringify!($rest), "`")*);
    };
}

frozen_primitive_catalog!(
    Unit,
    Bool,
    Char,
    SignedInteger(IntegerWidth),
    UnsignedInteger(IntegerWidth),
    BinaryFloat(FloatWidth),
    Decimal,
    String,
    Null,
    Undefined,
);

/// ADR-009 B1 S1 — single-source list of the `FrozenType` payload variants
/// that are constructable TODAY (their payload-descriptor tickets have
/// landed). Generates the array the descriptor-schema registration consumes,
/// the doc enumeration, and the `reflect` row description — one list, no
/// drift.
///
/// Central design call (Dec 50/94 tension, logged in `docs/defections.md`):
/// the injected `FrozenType` enum declares ONLY the constructable payload
/// variants; the 10-category layer stays exhaustive via
/// [`FrozenTypeCategory`]. Each later B ticket extends the enum — a
/// sanctioned comptime-ABI change per spec §3.3. Variant ids stay pinned to
/// the catalog ordinals via [`FrozenTypeCategory::catalog_ordinal`].
macro_rules! frozen_type_enabled_payload_catalog {
    ($first:ident $(, $rest:ident)* $(,)?) => {
        /// The payload variants of `FrozenType` whose descriptor tickets
        /// have landed. Reflecting any OTHER category is a named
        /// compile-time rejection — never a partial descriptor.
        pub const FROZEN_TYPE_ENABLED_PAYLOAD_CATEGORIES:
            [FrozenTypeCategory; [stringify!($first) $(, stringify!($rest))*].len()] =
            [FrozenTypeCategory::$first $(, FrozenTypeCategory::$rest)*];

        /// Compile-time rendering of the enabled-payload enumeration,
        /// generated from the same list the schema registration consumes.
        pub const FROZEN_TYPE_ENABLED_PAYLOADS_DOC: &str =
            concat!("`", stringify!($first), "`" $(, " | `", stringify!($rest), "`")*);

        /// ADR-009 B1 S3: the payload descriptor TYPE name for an ENABLED
        /// payload category (`Frozen` + category name — `FrozenPrimitive` /
        /// `FrozenNever` / `FrozenErased`), generated from the same enabled
        /// list. A non-enabled category has no descriptor type — its
        /// `reflect()` is the named R1 per-category rejection, never a
        /// partial descriptor. Consumed by the mini-VM payload-model item
        /// injection and the outer type-check; no hand-written duplicate.
        pub const fn frozen_type_enabled_payload_type_name(
            category: FrozenTypeCategory,
        ) -> Option<&'static str> {
            match category {
                FrozenTypeCategory::$first => Some(concat!("Frozen", stringify!($first))),
                $(FrozenTypeCategory::$rest => Some(concat!("Frozen", stringify!($rest))),)*
                _ => None,
            }
        }

        /// Description text for the LSP-visible `reflect` builtin row. The
        /// enabled-payload enumeration and the stage note are generated from
        /// the enabled-payload list — never hand-written.
        const REFLECT_ROW_DESCRIPTION: &str = concat!(
            "Reflect an opaque TypeRef into the sealed FrozenType payload sum. \
             Enabled payload variants (stage 2): ",
            "`", stringify!($first), "`" $(, " | `", stringify!($rest), "`")*,
            ". Reflecting a category whose payload ticket has not landed is a \
             named compile-time rejection (use type_category for the exhaustive \
             category). Only valid inside comptime blocks."
        );
    };
}

frozen_type_enabled_payload_catalog!(Primitive, Never, Erased);

/// ADR-009 B1 S3: the SPELLABLE name of the payload-model enum the comptime
/// mini-VM injects for `reflect()` results (`match FrozenType::Primitive(p)
/// { ... }`). The VALUE carrier stays the unspellable descriptor schema
/// (`builtin_schemas::COMPTIME_FROZEN_TYPE_SCHEMA`); this name exists so the
/// item injection, the `register_enum` variant-id pin, the outer type-check
/// return type, and the lift wall all reference ONE constant.
pub const FROZEN_TYPE_PAYLOAD_ENUM_NAME: &str = "FrozenType";

/// ADR-009 B1 S3: variant-id pin for the comptime-injected `FrozenType`
/// payload enum. Enabled variant names map to their Dec 50/94 catalog
/// ORDINALS (Primitive=0, Never=1, Erased=9) — the same ids the unspellable
/// descriptor schema registration uses — so the spellable model enum and
/// the value carrier can never disagree, and later B tickets add variants
/// without renumbering (comptime-ABI stability, spec §3.3). Any other name
/// has no pin: reflecting a non-enabled category is the named R1 rejection
/// upstream, never a dense re-id here.
pub fn frozen_type_payload_variant_ordinal(variant_name: &str) -> Option<u16> {
    FROZEN_TYPE_ENABLED_PAYLOAD_CATEGORIES
        .into_iter()
        .find(|category| category.variant_name() == variant_name)
        .map(FrozenTypeCategory::catalog_ordinal)
}

/// ADR-009 B1 S5 — catalog-keyed variant lookup for the reflection enums the
/// LSP completes (`FrozenTypeCategory`, the enabled `FrozenType` payload sum,
/// `FrozenPrimitive`, and the width-domain enums). Every returned list is
/// generated from the same shared catalog constants the compiler consumes
/// (`FrozenTypeCategory::ALL`, [`FROZEN_TYPE_ENABLED_PAYLOAD_CATEGORIES`],
/// [`FROZEN_PRIMITIVE_VARIANTS`], `IntegerWidth::ALL`, `FloatWidth::ALL`) —
/// deliberately no second hand-written variant list (the exact defect the A1
/// close-out deleted), and no Unknown/Any arm anywhere. The `FrozenType`
/// payload sum completes ONLY its enabled payload variants; non-enabled
/// categories have no completion arm just as they have no descriptor.
/// Returns `None` for names outside the reflection catalog (including the
/// `FrozenNever`/`FrozenErased` payload STRUCTS, which have no variants) so
/// callers fall back to user-defined enums.
pub fn reflection_enum_variant_names(enum_name: &str) -> Option<Vec<&'static str>> {
    if enum_name == "FrozenTypeCategory" {
        return Some(
            FrozenTypeCategory::ALL
                .into_iter()
                .map(FrozenTypeCategory::variant_name)
                .collect(),
        );
    }
    if enum_name == FROZEN_TYPE_PAYLOAD_ENUM_NAME {
        return Some(
            FROZEN_TYPE_ENABLED_PAYLOAD_CATEGORIES
                .into_iter()
                .map(FrozenTypeCategory::variant_name)
                .collect(),
        );
    }
    // The only enabled payload descriptor TYPE that is itself an enum is
    // `FrozenPrimitive` (`FrozenNever`/`FrozenErased` are structs); its name
    // comes from the catalog-generated type-name derivation, never a second
    // literal.
    if Some(enum_name) == frozen_type_enabled_payload_type_name(FrozenTypeCategory::Primitive) {
        return Some(FROZEN_PRIMITIVE_VARIANTS.iter().map(|v| v.name).collect());
    }
    if enum_name == INTEGER_WIDTH_SCHEMA_NAME {
        return Some(
            IntegerWidth::ALL
                .into_iter()
                .map(IntegerWidth::variant_name)
                .collect(),
        );
    }
    if enum_name == FLOAT_WIDTH_SCHEMA_NAME {
        return Some(
            FloatWidth::ALL
                .into_iter()
                .map(FloatWidth::variant_name)
                .collect(),
        );
    }
    None
}

/// LSP-visible builtin row for `reflect`, owned by the shared reflection
/// catalog (ADR-009 B1 S1). Its enabled-payload enumeration is generated
/// from the same list as [`FROZEN_TYPE_ENABLED_PAYLOAD_CATEGORIES`];
/// `builtin_metadata::CORE_BUILTINS` includes this descriptor verbatim.
pub const REFLECT_BUILTIN_ROW: BuiltinMetadata = BuiltinMetadata {
    name: "reflect",
    signature: "reflect(type_ref: TypeRef<T>) -> FrozenType<T>",
    description: REFLECT_ROW_DESCRIPTION,
    category: "Comptime",
    parameters: &[BuiltinParam {
        name: "type_ref",
        param_type: "TypeRef<T>",
        optional: false,
        description: "Compiler-issued type identity",
    }],
    return_type: "FrozenType<T>",
    example: Some("comptime { reflect(type_ref(int)) }"),
};

/// LSP-visible builtin row for `type_ref`, owned by the shared reflection
/// catalog (ADR-009 A1 S4). `builtin_metadata::CORE_BUILTINS` includes this
/// descriptor verbatim; it must not be duplicated as a hand-written row.
pub const TYPE_REF_BUILTIN_ROW: BuiltinMetadata = BuiltinMetadata {
    name: "type_ref",
    signature: "type_ref(T) -> TypeRef<T>",
    description: "Create an opaque compiler-issued identity for type syntax. Strings cannot construct a TypeRef. Only valid inside comptime blocks.",
    category: "Comptime",
    parameters: &[BuiltinParam {
        name: "T",
        param_type: "type",
        optional: false,
        description: "A type resolved by the compiler",
    }],
    return_type: "TypeRef<T>",
    example: Some("comptime { type_ref(Point) }"),
};

/// LSP-visible builtin row for `type_category`, owned by the shared
/// reflection catalog (ADR-009 A1 S4). Its category enumeration is generated
/// from the same variant list as [`FrozenTypeCategory::ALL`].
pub const TYPE_CATEGORY_BUILTIN_ROW: BuiltinMetadata = BuiltinMetadata {
    name: "type_category",
    signature: "type_category(type_ref: TypeRef<T>) -> FrozenTypeCategory",
    description: TYPE_CATEGORY_ROW_DESCRIPTION,
    category: "Comptime",
    parameters: &[BuiltinParam {
        name: "type_ref",
        param_type: "TypeRef<T>",
        optional: false,
        description: "Compiler-issued type identity",
    }],
    return_type: "FrozenTypeCategory",
    example: Some("comptime { type_category(type_ref(Point)) }"),
};

/// Return a diagnostic when a compile-stage capability is about to be baked
/// into ordinary runtime code. Scalar comptime results remain liftable; typed
/// reflection capabilities must be consumed before the stage boundary.
pub fn runtime_lift_rejection(value: &KindedSlot) -> Option<&'static str> {
    if value.kind() != NativeKind::Ptr(HeapKind::TypedObject) {
        return None;
    }
    let storage = value.as_typed_object_storage()?;
    let schema = crate::type_schema::lookup_schema_by_id_public(storage.schema_id as u32)?;
    match schema.name.as_str() {
        COMPTIME_FROZEN_TYPE_REF_SCHEMA => {
            Some("TypeRef is a comptime-only compiler capability and cannot enter runtime code")
        }
        "FrozenTypeCategory" => Some(
            "FrozenTypeCategory is comptime-only reflection data and cannot enter runtime code",
        ),
        // ADR-009 B1 S1 — one named arm per payload-descriptor schema,
        // registered in the SAME commit as the schema itself, so no
        // descriptor schema ever exists without a lift wall. The B1 S3
        // SPELLABLE payload-model names the mini-VM injects (the
        // `FrozenTypeCategory` precedent) share each arm — a lookalike
        // value carrying the injected model schema is comptime-only
        // reflection data exactly like the unspellable carrier. The
        // spellable names are pinned to the catalog-generated
        // `frozen_type_enabled_payload_type_name` by unit test.
        COMPTIME_FROZEN_TYPE_SCHEMA | FROZEN_TYPE_PAYLOAD_ENUM_NAME => {
            Some("FrozenType is comptime-only reflection data and cannot enter runtime code")
        }
        COMPTIME_FROZEN_PRIMITIVE_SCHEMA | "FrozenPrimitive" => {
            Some("FrozenPrimitive is comptime-only reflection data and cannot enter runtime code")
        }
        COMPTIME_FROZEN_NEVER_SCHEMA | "FrozenNever" => {
            Some("FrozenNever is comptime-only reflection data and cannot enter runtime code")
        }
        COMPTIME_FROZEN_ERASED_SCHEMA | "FrozenErased" => {
            Some("FrozenErased is comptime-only reflection data and cannot enter runtime code")
        }
        // ADR-009 B1 S2 — the width-domain enum carriers (the
        // `FrozenPrimitive` family payloads) are comptime-only reflection
        // data too; arms registered in the SAME commit as their schemas.
        INTEGER_WIDTH_SCHEMA_NAME => {
            Some("IntegerWidth is comptime-only reflection data and cannot enter runtime code")
        }
        FLOAT_WIDTH_SCHEMA_NAME => {
            Some("FloatWidth is comptime-only reflection data and cannot enter runtime code")
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn frozen_type_category_catalog_is_complete_ordered_and_unique() {
        let names: Vec<_> = FrozenTypeCategory::ALL
            .into_iter()
            .map(FrozenTypeCategory::variant_name)
            .collect();
        assert_eq!(
            names,
            [
                "Primitive",
                "Never",
                "Parameter",
                "Nominal",
                "Tuple",
                "Record",
                "Callable",
                "Reference",
                "Union",
                "Erased",
            ]
        );
        assert_eq!(names.iter().copied().collect::<HashSet<_>>().len(), 10);
        assert!(!names.contains(&"Unknown"));
    }

    /// The compile-time-generated variants doc must be byte-identical to a
    /// runtime derivation from `ALL` — proving both expand from the single
    /// source list (no drift possible).
    #[test]
    fn variants_doc_is_derived_from_the_shared_catalog() {
        let derived = FrozenTypeCategory::ALL
            .into_iter()
            .map(|category| format!("`{}`", category.variant_name()))
            .collect::<Vec<_>>()
            .join(" | ");
        assert_eq!(FROZEN_TYPE_CATEGORY_VARIANTS_DOC, derived);
    }

    /// The LSP-visible `type_category` row embeds the generated catalog
    /// enumeration and keeps the load-bearing hover phrase asserted by the
    /// public LSP matrix (`tests/lsp/typed_comptime.rs`).
    #[test]
    fn type_category_row_description_embeds_the_generated_catalog() {
        assert!(
            TYPE_CATEGORY_BUILTIN_ROW
                .description
                .contains(FROZEN_TYPE_CATEGORY_VARIANTS_DOC),
            "description: {}",
            TYPE_CATEGORY_BUILTIN_ROW.description
        );
        assert!(
            TYPE_CATEGORY_BUILTIN_ROW
                .description
                .contains("exhaustive semantic category"),
        );
        assert_eq!(TYPE_CATEGORY_BUILTIN_ROW.name, "type_category");
        assert_eq!(TYPE_CATEGORY_BUILTIN_ROW.category, "Comptime");
        assert_eq!(TYPE_CATEGORY_BUILTIN_ROW.return_type, "FrozenTypeCategory");
    }

    /// The `type_ref` row is catalog-owned and keeps the load-bearing hover
    /// phrases ("opaque compiler-issued identity", the typed signature).
    #[test]
    fn type_ref_row_is_catalog_owned_with_typed_signature() {
        assert_eq!(TYPE_REF_BUILTIN_ROW.name, "type_ref");
        assert_eq!(TYPE_REF_BUILTIN_ROW.signature, "type_ref(T) -> TypeRef<T>");
        assert!(
            TYPE_REF_BUILTIN_ROW
                .description
                .contains("opaque compiler-issued identity")
        );
        assert_eq!(TYPE_REF_BUILTIN_ROW.category, "Comptime");
    }

    // ── ADR-009 B1 S1: sealed FrozenPrimitive sub-algebra catalog ────────

    /// The FrozenPrimitive catalog is the Dec 50/94 sealed sub-algebra:
    /// unit, bool, char, the signed/unsigned integer families, the binary
    /// float family, exact decimal, string, null, undefined — complete,
    /// ordered, unique, with no unknown/any escape arm. Width/domain data
    /// is typed payload on the family variants only.
    #[test]
    fn frozen_primitive_catalog_is_complete_ordered_and_unique() {
        let names: Vec<_> = FROZEN_PRIMITIVE_VARIANTS
            .iter()
            .map(|variant| variant.name)
            .collect();
        assert_eq!(
            names,
            [
                "Unit",
                "Bool",
                "Char",
                "SignedInteger",
                "UnsignedInteger",
                "BinaryFloat",
                "Decimal",
                "String",
                "Null",
                "Undefined",
            ]
        );
        assert_eq!(names.iter().copied().collect::<HashSet<_>>().len(), 10);
        assert!(!names.contains(&"Unknown"));
        assert!(!names.contains(&"Any"));

        // Exactly the integer/float FAMILY variants carry a width/domain
        // payload; scalar members carry none.
        for variant in FROZEN_PRIMITIVE_VARIANTS {
            let expected = matches!(
                variant.name,
                "SignedInteger" | "UnsignedInteger" | "BinaryFloat"
            ) as u16;
            assert_eq!(
                variant.payload_arity, expected,
                "payload arity for {}",
                variant.name
            );
        }
    }

    /// The compile-time-generated FrozenPrimitive variants doc must be
    /// byte-identical to a runtime derivation from the descriptor table —
    /// proving both expand from the single source list (no drift possible).
    #[test]
    fn frozen_primitive_variants_doc_is_derived_from_the_shared_catalog() {
        let derived = FROZEN_PRIMITIVE_VARIANTS
            .iter()
            .map(|variant| format!("`{}`", variant.name))
            .collect::<Vec<_>>()
            .join(" | ");
        assert_eq!(FROZEN_PRIMITIVE_VARIANTS_DOC, derived);
    }

    /// One constructed value per FrozenPrimitive variant: `variant_name`
    /// must match the descriptor table in order — the enum and the table
    /// expand from the same macro token list.
    #[test]
    fn frozen_primitive_variant_names_match_the_descriptor_table() {
        let values = [
            FrozenPrimitive::Unit,
            FrozenPrimitive::Bool,
            FrozenPrimitive::Char,
            FrozenPrimitive::SignedInteger(IntegerWidth::W64),
            FrozenPrimitive::UnsignedInteger(IntegerWidth::W32),
            FrozenPrimitive::BinaryFloat(FloatWidth::W64),
            FrozenPrimitive::Decimal,
            FrozenPrimitive::String,
            FrozenPrimitive::Null,
            FrozenPrimitive::Undefined,
        ];
        assert_eq!(values.len(), FROZEN_PRIMITIVE_VARIANTS.len());
        for (value, descriptor) in values.iter().zip(FROZEN_PRIMITIVE_VARIANTS.iter()) {
            assert_eq!(value.variant_name(), descriptor.name);
        }
    }

    /// ADR-009 B1 S2: the width-domain enums expose catalog variant names
    /// (single source: the enum definitions + `ALL` tables), driving the
    /// `IntegerWidth`/`FloatWidth` descriptor-schema registrations — no
    /// second hand-written variant list.
    #[test]
    fn width_domain_enums_expose_catalog_variant_names() {
        assert_eq!(
            IntegerWidth::ALL.map(IntegerWidth::variant_name),
            ["W8", "W16", "W32", "W64", "Arbitrary"]
        );
        assert_eq!(
            FloatWidth::ALL.map(FloatWidth::variant_name),
            ["W32", "W64"]
        );
        assert_eq!(INTEGER_WIDTH_SCHEMA_NAME, "IntegerWidth");
        assert_eq!(FLOAT_WIDTH_SCHEMA_NAME, "FloatWidth");
    }

    /// Named decision (B1 S1, logged in docs/defections.md): `bigint` is a
    /// member of the signed integer family with an explicit `Arbitrary`
    /// width-domain member — not its own catalog bucket and not an
    /// "unresolved" placeholder.
    #[test]
    fn integer_width_domain_includes_an_explicit_arbitrary_member_for_bigint() {
        assert_eq!(
            IntegerWidth::ALL,
            [
                IntegerWidth::W8,
                IntegerWidth::W16,
                IntegerWidth::W32,
                IntegerWidth::W64,
                IntegerWidth::Arbitrary,
            ]
        );
        assert_eq!(FloatWidth::ALL, [FloatWidth::W32, FloatWidth::W64]);
        assert_eq!(
            FrozenPrimitive::SignedInteger(IntegerWidth::Arbitrary).variant_name(),
            "SignedInteger"
        );
    }

    // ── ADR-009 B1 S1: enabled FrozenType payload variants ───────────────

    /// The injected `FrozenType` enum declares ONLY the payload variants
    /// whose descriptor tickets have landed (B1: Primitive/Never/Erased).
    /// Variant ids are pinned to the Dec 50/94 10-variant catalog ORDINALS
    /// (Primitive=0, Never=1, Erased=9) — never densely renumbered — so
    /// later B tickets add variants without changing existing ids
    /// (comptime-ABI stability, spec §3.3). The 10-variant
    /// `FrozenTypeCategory` catalog itself is NOT touched; its completeness
    /// test above is the canary.
    #[test]
    fn enabled_payload_variants_are_ordinal_pinned_to_the_category_catalog() {
        let enabled: Vec<_> = FROZEN_TYPE_ENABLED_PAYLOAD_CATEGORIES
            .into_iter()
            .map(|category| (category.variant_name(), category.catalog_ordinal()))
            .collect();
        assert_eq!(enabled, [("Primitive", 0), ("Never", 1), ("Erased", 9)]);

        // Ordinals ARE the catalog declaration positions.
        for category in FROZEN_TYPE_ENABLED_PAYLOAD_CATEGORIES {
            let position = FrozenTypeCategory::ALL
                .iter()
                .position(|c| *c == category)
                .expect("enabled payload variant must be a catalog category");
            assert_eq!(position as u16, category.catalog_ordinal());
        }
        assert_eq!(FrozenTypeCategory::ALL.len(), 10);
    }

    /// The enabled-payload doc enumeration is derived from the same list
    /// the schema registration consumes.
    #[test]
    fn enabled_payloads_doc_is_derived_from_the_enabled_list() {
        let derived = FROZEN_TYPE_ENABLED_PAYLOAD_CATEGORIES
            .into_iter()
            .map(|category| format!("`{}`", category.variant_name()))
            .collect::<Vec<_>>()
            .join(" | ");
        assert_eq!(FROZEN_TYPE_ENABLED_PAYLOADS_DOC, derived);
    }

    /// The LSP-visible `reflect` row is catalog-owned, keeps the typed
    /// signature, and embeds the generated enabled-payload enumeration plus
    /// the stage note (non-enabled categories reject at compile time).
    #[test]
    fn reflect_row_is_catalog_owned_with_enabled_payload_enumeration() {
        assert_eq!(REFLECT_BUILTIN_ROW.name, "reflect");
        assert_eq!(
            REFLECT_BUILTIN_ROW.signature,
            "reflect(type_ref: TypeRef<T>) -> FrozenType<T>"
        );
        assert_eq!(REFLECT_BUILTIN_ROW.category, "Comptime");
        assert_eq!(REFLECT_BUILTIN_ROW.return_type, "FrozenType<T>");
        assert!(
            REFLECT_BUILTIN_ROW
                .description
                .contains(FROZEN_TYPE_ENABLED_PAYLOADS_DOC),
            "description must embed the generated enabled-payload enumeration: {}",
            REFLECT_BUILTIN_ROW.description
        );
        assert!(
            REFLECT_BUILTIN_ROW
                .description
                .contains("named compile-time rejection"),
            "description must carry the stage note: {}",
            REFLECT_BUILTIN_ROW.description
        );
        assert!(
            REFLECT_BUILTIN_ROW
                .description
                .contains("Only valid inside comptime blocks")
        );
    }

    // ── ADR-009 B1 S1: runtime lift-rejection wall ────────────────────────

    mod lift_rejection {
        use super::*;
        use crate::type_schema::TypeSchemaRegistry;
        use crate::type_schema::builtin_schemas::{
            COMPTIME_FROZEN_ERASED_SCHEMA, COMPTIME_FROZEN_NEVER_SCHEMA,
            COMPTIME_FROZEN_PRIMITIVE_SCHEMA, COMPTIME_FROZEN_TYPE_SCHEMA,
            register_builtin_schemas,
        };
        use crate::type_schema::current::SyncRegistryScope;
        use shape_value::TypedObjectStorage;
        use std::sync::Arc;

        /// Fabricate a `KindedSlot` carrying a zero-field
        /// `TypedObjectStorage` stamped with `schema_name`'s id (v2-raw
        /// `_new` allocator; the `KindedSlot` owns the single refcount share
        /// and retires it via its kind-dispatched `Drop`), install the
        /// registry as the ambient scope so the id resolves, run `check`.
        fn with_fabricated_descriptor_slot(schema_name: &str, check: impl FnOnce(&KindedSlot)) {
            let mut registry = TypeSchemaRegistry::new();
            register_builtin_schemas(&mut registry);
            let schema_id = registry
                .get(schema_name)
                .unwrap_or_else(|| panic!("schema {schema_name:?} must be registered"))
                .id;
            let _scope = SyncRegistryScope::enter(Arc::new(registry));
            let ptr = TypedObjectStorage::_new(
                u64::from(schema_id),
                Vec::new().into_boxed_slice(),
                0,
                Vec::new().into(),
            );
            let slot = KindedSlot::from_typed_object_raw(ptr);
            check(&slot);
        }

        /// Every B1 descriptor schema has its own named lift-rejection arm —
        /// registered in the SAME commit as the schema itself, so no schema
        /// ever exists without a lift wall.
        #[test]
        fn each_new_descriptor_schema_has_its_own_named_lift_rejection() {
            for (schema, expected) in [
                (
                    COMPTIME_FROZEN_TYPE_SCHEMA,
                    "FrozenType is comptime-only reflection data and cannot enter runtime code",
                ),
                (
                    COMPTIME_FROZEN_PRIMITIVE_SCHEMA,
                    "FrozenPrimitive is comptime-only reflection data and cannot enter runtime code",
                ),
                (
                    COMPTIME_FROZEN_NEVER_SCHEMA,
                    "FrozenNever is comptime-only reflection data and cannot enter runtime code",
                ),
                (
                    COMPTIME_FROZEN_ERASED_SCHEMA,
                    "FrozenErased is comptime-only reflection data and cannot enter runtime code",
                ),
            ] {
                with_fabricated_descriptor_slot(schema, |slot| {
                    assert_eq!(runtime_lift_rejection(slot), Some(expected));
                });
            }
        }

        /// ADR-009 B1 S2: the width-domain enum carriers (`IntegerWidth` /
        /// `FloatWidth`, the `FrozenPrimitive` family payloads) are
        /// comptime-only reflection data too — each registered with its own
        /// named lift-rejection arm in the SAME commit as its schema.
        #[test]
        fn width_domain_schemas_have_their_own_named_lift_rejection() {
            for (schema, expected) in [
                (
                    INTEGER_WIDTH_SCHEMA_NAME,
                    "IntegerWidth is comptime-only reflection data and cannot enter runtime code",
                ),
                (
                    FLOAT_WIDTH_SCHEMA_NAME,
                    "FloatWidth is comptime-only reflection data and cannot enter runtime code",
                ),
            ] {
                with_fabricated_descriptor_slot(schema, |slot| {
                    assert_eq!(runtime_lift_rejection(slot), Some(expected));
                });
            }
        }

        /// The pre-existing A1 arms keep firing after the B1 extension.
        #[test]
        fn lift_rejection_still_fires_for_type_ref_and_frozen_type_category() {
            with_fabricated_descriptor_slot(COMPTIME_FROZEN_TYPE_REF_SCHEMA, |slot| {
                assert_eq!(
                    runtime_lift_rejection(slot),
                    Some(
                        "TypeRef is a comptime-only compiler capability and cannot enter runtime code"
                    )
                );
            });
            with_fabricated_descriptor_slot("FrozenTypeCategory", |slot| {
                assert_eq!(
                    runtime_lift_rejection(slot),
                    Some(
                        "FrozenTypeCategory is comptime-only reflection data and cannot enter runtime code"
                    )
                );
            });
        }

        /// Ordinary typed objects and scalars remain liftable.
        #[test]
        fn lift_rejection_ignores_ordinary_values() {
            with_fabricated_descriptor_slot("__Option", |slot| {
                assert_eq!(runtime_lift_rejection(slot), None);
            });
            assert_eq!(runtime_lift_rejection(&KindedSlot::from_int(7)), None);
            assert_eq!(runtime_lift_rejection(&KindedSlot::from_bool(true)), None);
        }

        /// ADR-009 B1 S3: the SPELLABLE payload-model names the mini-VM
        /// injects (`FrozenType` + the payload descriptor type names) get
        /// their own lift arms in the SAME commit as the injection — a
        /// value carrying a mini-VM-registered spellable model schema is
        /// comptime-only reflection data exactly like the unspellable
        /// carrier (the `FrozenTypeCategory` precedent).
        #[test]
        fn spellable_payload_model_names_have_their_own_named_lift_rejection() {
            let mut registry = TypeSchemaRegistry::new();
            register_builtin_schemas(&mut registry);
            // Mirror the mini-VM injected model registrations (spellable
            // names; contents irrelevant to the name-matched wall).
            for name in [
                FROZEN_TYPE_PAYLOAD_ENUM_NAME,
                "FrozenPrimitive",
                "FrozenNever",
                "FrozenErased",
            ] {
                registry.register_type_scoped(name, Vec::new());
            }
            let ids: Vec<(u32, &str)> = [
                (
                    FROZEN_TYPE_PAYLOAD_ENUM_NAME,
                    "FrozenType is comptime-only reflection data and cannot enter runtime code",
                ),
                (
                    "FrozenPrimitive",
                    "FrozenPrimitive is comptime-only reflection data and cannot enter runtime code",
                ),
                (
                    "FrozenNever",
                    "FrozenNever is comptime-only reflection data and cannot enter runtime code",
                ),
                (
                    "FrozenErased",
                    "FrozenErased is comptime-only reflection data and cannot enter runtime code",
                ),
            ]
            .into_iter()
            .map(|(name, expected)| {
                (
                    registry.get(name).expect("registered above").id,
                    expected,
                )
            })
            .collect();
            let _scope = SyncRegistryScope::enter(Arc::new(registry));
            for (schema_id, expected) in ids {
                let ptr = TypedObjectStorage::_new(
                    u64::from(schema_id),
                    Vec::new().into_boxed_slice(),
                    0,
                    Vec::new().into(),
                );
                let slot = KindedSlot::from_typed_object_raw(ptr);
                assert_eq!(runtime_lift_rejection(&slot), Some(expected));
            }
        }
    }

    // ── ADR-009 B1 S3: injected payload-model single-source constants ─────

    /// The spellable payload-model enum name + the per-enabled-category
    /// descriptor type names are generated from the shared catalog
    /// (`Frozen` + category name) — the mini-VM item injection, the outer
    /// type-check return type, and the lift wall all consume THESE, never a
    /// hand-written duplicate list.
    #[test]
    fn enabled_payload_descriptor_type_names_derive_from_the_catalog() {
        assert_eq!(FROZEN_TYPE_PAYLOAD_ENUM_NAME, "FrozenType");
        let derived: Vec<_> = FROZEN_TYPE_ENABLED_PAYLOAD_CATEGORIES
            .into_iter()
            .map(|category| {
                frozen_type_enabled_payload_type_name(category)
                    .expect("every enabled category carries a descriptor type name")
            })
            .collect();
        assert_eq!(derived, ["FrozenPrimitive", "FrozenNever", "FrozenErased"]);
        for category in FrozenTypeCategory::ALL {
            let enabled = FROZEN_TYPE_ENABLED_PAYLOAD_CATEGORIES.contains(&category);
            assert_eq!(
                frozen_type_enabled_payload_type_name(category).is_some(),
                enabled,
                "descriptor type name must exist exactly for enabled categories ({})",
                category.variant_name()
            );
        }
    }

    /// The comptime-injected `FrozenType` payload enum's variant-id pin:
    /// enabled variant names map to their Dec 50/94 catalog ORDINALS
    /// (Primitive=0, Never=1, Erased=9); every other name (pending
    /// categories, unknown names) has NO pin. The compiler's
    /// `register_enum` consumes this for the injected model so the
    /// spellable enum's ids can never drift dense while the unspellable
    /// descriptor schema stays ordinal-pinned.
    #[test]
    fn frozen_type_payload_variant_ordinals_are_catalog_pinned() {
        assert_eq!(frozen_type_payload_variant_ordinal("Primitive"), Some(0));
        assert_eq!(frozen_type_payload_variant_ordinal("Never"), Some(1));
        assert_eq!(frozen_type_payload_variant_ordinal("Erased"), Some(9));
        for pending in [
            "Parameter",
            "Nominal",
            "Tuple",
            "Record",
            "Callable",
            "Reference",
            "Union",
            "Unknown",
            "Any",
        ] {
            assert_eq!(
                frozen_type_payload_variant_ordinal(pending),
                None,
                "{pending} must not be pinnable"
            );
        }
    }

    // ── ADR-009 B1 S5: catalog-keyed LSP variant lookup ───────────────────

    /// Every reflection-enum variant list the LSP consumes is generated from
    /// the shared catalog constants — byte-identical to a direct derivation,
    /// closed (no Unknown/Any arm), and the `FrozenType` payload sum offers
    /// ONLY its enabled payload variants.
    #[test]
    fn reflection_enum_variant_lookup_is_catalog_keyed_and_closed() {
        assert_eq!(
            reflection_enum_variant_names("FrozenTypeCategory"),
            Some(
                FrozenTypeCategory::ALL
                    .into_iter()
                    .map(FrozenTypeCategory::variant_name)
                    .collect()
            )
        );
        assert_eq!(
            reflection_enum_variant_names(FROZEN_TYPE_PAYLOAD_ENUM_NAME),
            Some(
                FROZEN_TYPE_ENABLED_PAYLOAD_CATEGORIES
                    .into_iter()
                    .map(FrozenTypeCategory::variant_name)
                    .collect()
            )
        );
        assert_eq!(
            reflection_enum_variant_names(FROZEN_TYPE_PAYLOAD_ENUM_NAME),
            Some(vec!["Primitive", "Never", "Erased"]),
            "the FrozenType payload sum completes only enabled payload variants"
        );
        assert_eq!(
            reflection_enum_variant_names("FrozenPrimitive"),
            Some(FROZEN_PRIMITIVE_VARIANTS.iter().map(|v| v.name).collect())
        );
        assert_eq!(
            reflection_enum_variant_names(INTEGER_WIDTH_SCHEMA_NAME),
            Some(
                IntegerWidth::ALL
                    .into_iter()
                    .map(IntegerWidth::variant_name)
                    .collect()
            )
        );
        assert_eq!(
            reflection_enum_variant_names(FLOAT_WIDTH_SCHEMA_NAME),
            Some(
                FloatWidth::ALL
                    .into_iter()
                    .map(FloatWidth::variant_name)
                    .collect()
            )
        );
        for catalog_name in [
            "FrozenTypeCategory",
            FROZEN_TYPE_PAYLOAD_ENUM_NAME,
            "FrozenPrimitive",
            INTEGER_WIDTH_SCHEMA_NAME,
            FLOAT_WIDTH_SCHEMA_NAME,
        ] {
            let names = reflection_enum_variant_names(catalog_name)
                .expect("catalog names must resolve");
            assert!(!names.is_empty());
            assert!(!names.contains(&"Unknown"), "{catalog_name}");
            assert!(!names.contains(&"Any"), "{catalog_name}");
            assert!(!names.contains(&"InferenceVariable"), "{catalog_name}");
        }
        // Pending payload categories have NO completion arm on FrozenType.
        let frozen_type = reflection_enum_variant_names(FROZEN_TYPE_PAYLOAD_ENUM_NAME).unwrap();
        for category in FrozenTypeCategory::ALL {
            let enabled = FROZEN_TYPE_ENABLED_PAYLOAD_CATEGORIES.contains(&category);
            assert_eq!(
                frozen_type.contains(&category.variant_name()),
                enabled,
                "FrozenType completion arm for {}",
                category.variant_name()
            );
        }
    }

    /// Names outside the reflection catalog resolve to `None` — the payload
    /// STRUCTS (`FrozenNever`/`FrozenErased`) have no variants, and ordinary
    /// user enums fall back to program-derived members.
    #[test]
    fn reflection_enum_variant_lookup_ignores_non_catalog_names() {
        assert_eq!(reflection_enum_variant_names("FrozenNever"), None);
        assert_eq!(reflection_enum_variant_names("FrozenErased"), None);
        assert_eq!(reflection_enum_variant_names("Color"), None);
        assert_eq!(reflection_enum_variant_names(""), None);
    }

    /// The `FrozenPrimitive` descriptor table carries the typed
    /// width/domain payload TYPE per family variant (single macro source):
    /// exactly the arity-1 variants name a payload type, and the names are
    /// the width-domain schema names the mini-VM injects.
    #[test]
    fn frozen_primitive_descriptor_table_names_the_width_domain_payload_types() {
        for variant in FROZEN_PRIMITIVE_VARIANTS {
            let expected = match variant.name {
                "SignedInteger" | "UnsignedInteger" => Some(INTEGER_WIDTH_SCHEMA_NAME),
                "BinaryFloat" => Some(FLOAT_WIDTH_SCHEMA_NAME),
                _ => None,
            };
            assert_eq!(
                variant.payload_type, expected,
                "payload type for {}",
                variant.name
            );
            assert_eq!(
                variant.payload_type.is_some(),
                variant.payload_arity == 1,
                "payload type must exist exactly for arity-1 variants ({})",
                variant.name
            );
        }
    }
}
