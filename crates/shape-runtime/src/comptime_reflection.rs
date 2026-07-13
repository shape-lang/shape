use crate::builtin_metadata::{BuiltinMetadata, BuiltinParam};
use crate::type_schema::builtin_schemas::{
    COMPTIME_FROZEN_IMPL_REF_SCHEMA, COMPTIME_FROZEN_TRAIT_REF_SCHEMA,
    COMPTIME_FROZEN_TYPE_REF_SCHEMA,
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

/// LSP-visible builtin row for `trait_ref`, owned by the shared reflection
/// catalog (ADR-009 ticket B2, slice S3). `builtin_metadata::CORE_BUILTINS`
/// includes this descriptor verbatim; it must not be duplicated as a
/// hand-written row. Dec 49: a trait is a DISTINCT identity kind from a
/// value type — a TraitRef is never a TypeRef.
pub const TRAIT_REF_BUILTIN_ROW: BuiltinMetadata = BuiltinMetadata {
    name: "trait_ref",
    signature: "trait_ref(Tr) -> TraitRef<Tr>",
    description: "Create an opaque compiler-issued identity for a declared trait; a trait is not a value type, so a TraitRef is a distinct identity kind from TypeRef. Strings cannot construct a TraitRef. Only valid inside comptime blocks.",
    category: "Comptime",
    parameters: &[BuiltinParam {
        name: "Tr",
        param_type: "trait",
        optional: false,
        description: "A trait declared before the semantic-freeze barrier",
    }],
    return_type: "TraitRef<Tr>",
    example: Some("comptime { trait_ref(Serializable) }"),
};

/// LSP-visible builtin row for `find_impl`, owned by the shared reflection
/// catalog (ADR-009 ticket B2, slice S3). Evidence is branch-scoped and
/// compiler-issued (Dec 49): an unimplemented pair is `None` — never an
/// error and never partial evidence — and a boolean can never stand in for
/// an `ImplRef`.
pub const FIND_IMPL_BUILTIN_ROW: BuiltinMetadata = BuiltinMetadata {
    name: "find_impl",
    signature: "find_impl(type_ref: TypeRef<T>, trait_ref: TraitRef<Tr>) -> Option<ImplRef<T, Tr>>",
    description: "Look up compiler-issued implementation evidence for the exact (type, trait) identity pair frozen at the semantic-freeze barrier. An unimplemented pair is None — never an error and never partial evidence. A boolean cannot authorize an operation that requires implementation evidence. Only valid inside comptime blocks.",
    category: "Comptime",
    parameters: &[
        BuiltinParam {
            name: "type_ref",
            param_type: "TypeRef<T>",
            optional: false,
            description: "Compiler-issued type identity",
        },
        BuiltinParam {
            name: "trait_ref",
            param_type: "TraitRef<Tr>",
            optional: false,
            description: "Compiler-issued trait identity",
        },
    ],
    return_type: "Option<ImplRef<T, Tr>>",
    example: Some(
        "comptime { match find_impl(type_ref(User), trait_ref(Serializable)) { Some(proof) => /* consume evidence */, None => /* no impl */ } }",
    ),
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
        // ADR-009 (ticket B2, slice S3) rejection R6: trait identities and
        // implementation evidence never cross the stage boundary.
        COMPTIME_FROZEN_TRAIT_REF_SCHEMA => {
            Some("TraitRef is a comptime-only compiler capability and cannot enter runtime code")
        }
        COMPTIME_FROZEN_IMPL_REF_SCHEMA => Some(
            "ImplRef is comptime-only implementation evidence and cannot enter runtime code",
        ),
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

    /// ADR-009 (ticket B2, slice S3): the `trait_ref` row is catalog-owned
    /// with the load-bearing hover phrases — a trait is a DISTINCT identity
    /// kind, never a value type, and strings cannot construct a TraitRef.
    #[test]
    fn trait_ref_row_is_catalog_owned_with_typed_signature() {
        assert_eq!(TRAIT_REF_BUILTIN_ROW.name, "trait_ref");
        assert_eq!(
            TRAIT_REF_BUILTIN_ROW.signature,
            "trait_ref(Tr) -> TraitRef<Tr>"
        );
        assert_eq!(TRAIT_REF_BUILTIN_ROW.return_type, "TraitRef<Tr>");
        assert_eq!(TRAIT_REF_BUILTIN_ROW.category, "Comptime");
        assert!(
            TRAIT_REF_BUILTIN_ROW
                .description
                .contains("opaque compiler-issued identity")
        );
        assert!(
            TRAIT_REF_BUILTIN_ROW
                .description
                .contains("a trait is not a value type")
        );
        assert!(
            TRAIT_REF_BUILTIN_ROW
                .description
                .contains("Strings cannot construct a TraitRef")
        );
    }

    /// ADR-009 (ticket B2, slice S3): the `find_impl` row is catalog-owned;
    /// evidence is `Option<ImplRef<T, Tr>>` (unimplemented pair = `None`,
    /// never an error, never partial evidence) and a boolean can never
    /// authorize evidence-requiring generation (Dec 49).
    #[test]
    fn find_impl_row_is_catalog_owned_returning_optional_evidence() {
        assert_eq!(FIND_IMPL_BUILTIN_ROW.name, "find_impl");
        assert_eq!(
            FIND_IMPL_BUILTIN_ROW.signature,
            "find_impl(type_ref: TypeRef<T>, trait_ref: TraitRef<Tr>) -> Option<ImplRef<T, Tr>>"
        );
        assert_eq!(FIND_IMPL_BUILTIN_ROW.return_type, "Option<ImplRef<T, Tr>>");
        assert_eq!(FIND_IMPL_BUILTIN_ROW.category, "Comptime");
        assert!(
            FIND_IMPL_BUILTIN_ROW
                .description
                .contains("implementation evidence")
        );
        assert!(FIND_IMPL_BUILTIN_ROW.description.contains("None"));
        assert!(
            FIND_IMPL_BUILTIN_ROW
                .description
                .contains("boolean cannot authorize")
        );
    }

    /// ADR-009 (ticket B2, slice S3) rejection R6, A1 row-5 mirror: the
    /// reserved TraitRef/ImplRef carriers cannot lift past the stage
    /// boundary into runtime code — named diagnostics, keyed by the
    /// unspellable schema names. Scalars stay liftable.
    #[test]
    fn runtime_lift_rejection_names_trait_evidence_as_comptime_only() {
        use crate::type_schema::builtin_schemas::{
            COMPTIME_FROZEN_IMPL_REF_SCHEMA, COMPTIME_FROZEN_TRAIT_REF_SCHEMA,
        };
        use crate::type_schema::typed_object_for_named_schema;

        let trait_ref = typed_object_for_named_schema(
            COMPTIME_FROZEN_TRAIT_REF_SCHEMA,
            &[
                ("identity_high", KindedSlot::from_int(11)),
                ("identity_low", KindedSlot::from_int(22)),
            ],
        );
        let rejection = runtime_lift_rejection(&trait_ref).expect("TraitRef must not lift");
        assert!(
            rejection.contains("TraitRef is a comptime-only compiler capability"),
            "{rejection}"
        );
        assert!(rejection.contains("cannot enter runtime code"), "{rejection}");

        let impl_ref = typed_object_for_named_schema(
            COMPTIME_FROZEN_IMPL_REF_SCHEMA,
            &[
                ("trait_identity_high", KindedSlot::from_int(1)),
                ("trait_identity_low", KindedSlot::from_int(2)),
                ("type_identity_high", KindedSlot::from_int(3)),
                ("type_identity_low", KindedSlot::from_int(4)),
                ("impl_identity_high", KindedSlot::from_int(5)),
                ("impl_identity_low", KindedSlot::from_int(6)),
            ],
        );
        let rejection = runtime_lift_rejection(&impl_ref).expect("ImplRef must not lift");
        assert!(
            rejection.contains("ImplRef is comptime-only implementation evidence"),
            "{rejection}"
        );
        assert!(rejection.contains("cannot enter runtime code"), "{rejection}");

        // Scalar comptime results remain liftable.
        assert_eq!(runtime_lift_rejection(&KindedSlot::from_int(7)), None);
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
}
