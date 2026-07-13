use super::semantic_freeze::FreezeOverlay;
use crate::compiler::comptime_target;
use sha2::{Digest, Sha256};
use shape_ast::ast::TypeAnnotation;
pub(crate) use shape_runtime::comptime_reflection::FrozenTypeCategory;
use shape_runtime::type_schema::builtin_schemas::COMPTIME_FROZEN_TYPE_REF_SCHEMA;
use shape_runtime::type_schema::{current_registry, typed_object_for_named_schema};
use shape_value::heap_value::{HeapKind, HeapValue, TypedObjectPtr, TypedObjectStorage};
use shape_value::{KindedSlot, NativeKind};
use std::collections::HashMap;

/// Stable semantic identity carried by an opaque comptime `TypeRef`.
///
/// Two 64-bit fields preserve 128 bits of a canonical SHA-256 descriptor hash;
/// unlike a snapshot vector index, adding an unrelated type cannot renumber an
/// existing identity. Transparent aliases reuse the target identity directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct FrozenTypeIdentity {
    pub(crate) high: i64,
    pub(crate) low: i64,
}

impl FrozenTypeIdentity {
    pub(crate) const INVALID: Self = Self { high: -1, low: -1 };

    pub(super) fn from_canonical_descriptor(descriptor: &str) -> Self {
        let digest = Sha256::digest(descriptor.as_bytes());
        let high = i64::from_be_bytes(digest[0..8].try_into().expect("8-byte hash prefix"));
        let low = i64::from_be_bytes(digest[8..16].try_into().expect("8-byte hash suffix"));
        Self { high, low }
    }

    /// ADR-009 (ticket B2, slice S2; Dec 49 / Dec 50 rule 5): canonical TRAIT
    /// identity — a DISTINCT identity kind from value-type identities, keyed
    /// by the `trait:` descriptor prefix. Trait identities are NEVER interned
    /// into `FrozenTypeIndex.frozen_type_ids` (so `type_ref(TraitName)` keeps
    /// failing and `intern_identity`'s cross-category collision assertion
    /// never sees them) and there is deliberately NO
    /// `FrozenTypeCategory::Trait` variant.
    pub(super) fn for_trait(canonical_trait_name: &str) -> Self {
        Self::from_canonical_descriptor(&format!("trait:{canonical_trait_name}"))
    }

    /// ADR-009 (ticket B2, slice S2; Dec 49): canonical IMPL-evidence
    /// identity — `impl:{trait}:{type}:{impl_name_or_default}`, so canonical
    /// trait AND implementation identities enter the SHA-256 fingerprint
    /// scheme and named impls (`impl Trait for Type as Name`) are distinct
    /// evidence. `__default__` mirrors the registry's `DEFAULT_IMPL_NAME`
    /// selector convention (`environment/registry.rs`).
    pub(super) fn for_impl(
        canonical_trait_name: &str,
        target_type_name: &str,
        impl_name: Option<&str>,
    ) -> Self {
        Self::from_canonical_descriptor(&format!(
            "impl:{}:{}:{}",
            canonical_trait_name,
            target_type_name,
            impl_name.unwrap_or("__default__")
        ))
    }
}

/// ADR-009 §4.1 (ticket A1, slice S2): the semantic freeze's INTERNAL type
/// index. This is the reduced remainder of the deleted per-site
/// `TypeReflectionSnapshot` carrier (whose `build_type_reflection_snapshot`
/// per-site rebuild pattern S2 deleted): it survives only inside
/// [`super::semantic_freeze::SemanticFreeze`], never as a reachable parallel
/// carrier, and deliberately has no `Default`/empty constructor — the freeze
/// barrier is the single construction point. Scoped generic parameters live
/// in [`FreezeOverlay`], not here. Public `TypeRef` values carry only a
/// canonical semantic fingerprint, never a rendered type name or
/// index-local ordinal.
#[derive(Debug)]
pub(crate) struct FrozenTypeIndex {
    pub(crate) struct_defs: HashMap<String, Vec<(String, TypeAnnotation)>>,
    pub(crate) enum_defs: HashMap<String, Vec<String>>,
    pub(crate) alias_defs: HashMap<String, TypeAnnotation>,
    pub(crate) frozen_type_ids: HashMap<String, FrozenTypeIdentity>,
    pub(crate) frozen_type_categories: HashMap<FrozenTypeIdentity, FrozenTypeCategory>,
}

impl FrozenTypeIndex {
    pub(crate) fn frozen_type_id(&self, name: &str) -> Option<FrozenTypeIdentity> {
        self.frozen_type_ids.get(name).copied()
    }

    pub(super) fn category_for_identity(
        &self,
        identity: FrozenTypeIdentity,
    ) -> Result<FrozenTypeCategory, String> {
        self.frozen_type_categories
            .get(&identity)
            .copied()
            .ok_or_else(|| "type_ref received an unknown semantic type identity".to_string())
    }

    pub(super) fn rebuild_frozen_type_index(&mut self) {
        let mut ids = HashMap::new();
        let mut categories = HashMap::new();

        intern_synonyms(
            &mut ids,
            &mut categories,
            &["unit", "void", "()"],
            FrozenTypeCategory::Primitive,
        );
        for names in [
            &["bool"][..],
            &["char"][..],
            &["int", "i64"][..],
            &["i8"][..],
            &["i16"][..],
            &["i32"][..],
            &["u8"][..],
            &["u16"][..],
            &["u32"][..],
            &["u64"][..],
            &["bigint"][..],
            &["number", "f64", "float"][..],
            &["f32"][..],
            &["decimal"][..],
            &["string", "str"][..],
            &["null"][..],
            &["undefined"][..],
        ] {
            intern_synonyms(
                &mut ids,
                &mut categories,
                names,
                FrozenTypeCategory::Primitive,
            );
        }
        intern_synonyms(
            &mut ids,
            &mut categories,
            &["never"],
            FrozenTypeCategory::Never,
        );
        intern_synonyms(
            &mut ids,
            &mut categories,
            &["any"],
            FrozenTypeCategory::Erased,
        );

        for name in [
            "Array",
            "Vec",
            "HashMap",
            "Option",
            "Result",
            "Future",
            "Set",
            "Deque",
            "PriorityQueue",
            "Mutex",
            "Slice",
        ] {
            intern_identity(
                &mut ids,
                &mut categories,
                name,
                &format!("nominal:{name}"),
                FrozenTypeCategory::Nominal,
            );
        }

        let mut nominal_names: Vec<_> = self
            .struct_defs
            .keys()
            .chain(self.enum_defs.keys())
            .cloned()
            .collect();
        nominal_names.sort();
        nominal_names.dedup();
        for name in nominal_names {
            intern_identity(
                &mut ids,
                &mut categories,
                &name,
                &format!("nominal:{name}"),
                FrozenTypeCategory::Nominal,
            );
        }

        // Scoped generic parameters are NOT interned here: they enter through
        // a `FreezeOverlay` (`parameter:{owner}:{name}` identities layered
        // over the shared base), never through the base index (ADR-009 §4.1).

        // Aliases are transparent: an alias receives the exact identity of its
        // canonical target. Iterate to a fixed point so alias chains normalize.
        let mut aliases: Vec<_> = self.alias_defs.iter().collect();
        aliases.sort_by(|(left, _), (right, _)| left.cmp(right));
        for _ in 0..=aliases.len() {
            let mut changed = false;
            for (alias, target) in &aliases {
                let Some(target_name) = target.as_simple_name() else {
                    continue;
                };
                let Some(identity) = ids.get(target_name).copied() else {
                    continue;
                };
                changed |= ids.insert((*alias).clone(), identity) != Some(identity);
            }
            if !changed {
                break;
            }
        }

        self.frozen_type_ids = ids;
        self.frozen_type_categories = categories;
    }
}

fn intern_identity(
    ids: &mut HashMap<String, FrozenTypeIdentity>,
    categories: &mut HashMap<FrozenTypeIdentity, FrozenTypeCategory>,
    name: &str,
    canonical_descriptor: &str,
    category: FrozenTypeCategory,
) -> FrozenTypeIdentity {
    if let Some(identity) = ids.get(name) {
        return *identity;
    }
    let identity = FrozenTypeIdentity::from_canonical_descriptor(canonical_descriptor);
    if let Some(previous) = categories.insert(identity, category) {
        assert_eq!(
            previous, category,
            "canonical type identity collision across semantic categories"
        );
    }
    ids.insert(name.to_string(), identity);
    identity
}

fn intern_synonyms(
    ids: &mut HashMap<String, FrozenTypeIdentity>,
    categories: &mut HashMap<FrozenTypeIdentity, FrozenTypeCategory>,
    names: &[&str],
    category: FrozenTypeCategory,
) {
    let identity = intern_identity(
        ids,
        categories,
        names[0],
        &format!("{}:{}", category.variant_name(), names[0]),
        category,
    );
    for name in &names[1..] {
        ids.insert((*name).to_string(), identity);
    }
}

pub(crate) fn build_frozen_type_ref_heap_value(
    identity: FrozenTypeIdentity,
    freeze: &FreezeOverlay,
) -> Result<HeapValue, String> {
    // Rejection R1 (ADR-009 B2 slice S5, Dec 49): traits are not value
    // types. A frozen TRAIT identity (freeze input 4 — a distinct identity
    // kind, never interned into the type-identity map) reaching the TypeRef
    // builder is the NAMED trait rejection, not the generic
    // unknown-identity error a genuinely-unknown name keeps (A1 row 2).
    if freeze.is_frozen_trait_identity(identity) {
        return Err(super::trait_evidence::TRAIT_NOT_A_VALUE_TYPE_DIAGNOSTIC.to_string());
    }
    freeze.category_of(identity)?;
    typed_slot_into_heap_value(typed_object_for_named_schema(
        COMPTIME_FROZEN_TYPE_REF_SCHEMA,
        &[
            ("identity_high", KindedSlot::from_int(identity.high)),
            ("identity_low", KindedSlot::from_int(identity.low)),
        ],
    ))
}

pub(crate) fn frozen_type_category_from_ref(
    slot: &KindedSlot,
    freeze: &FreezeOverlay,
) -> Result<FrozenTypeCategory, String> {
    if slot.kind() != NativeKind::Ptr(HeapKind::TypedObject) {
        return Err("type_category expects a TypeRef value".to_string());
    }
    let storage = slot
        .as_typed_object_storage()
        .ok_or_else(|| "type_category received a null TypeRef value".to_string())?;
    let schema = shape_runtime::type_schema::lookup_schema_by_id_public(storage.schema_id as u32)
        .ok_or_else(|| {
        format!(
            "type_category could not resolve TypeRef schema id {}",
            storage.schema_id
        )
    })?;
    if schema.name != COMPTIME_FROZEN_TYPE_REF_SCHEMA {
        return Err(format!(
            "type_category expects TypeRef, got '{}'",
            schema.name
        ));
    }
    let identity_field = |name: &str| -> Result<i64, String> {
        let field = schema
            .get_field(name)
            .ok_or_else(|| format!("TypeRef schema has no {name} field"))?;
        storage
            .clone_field_kinded(field.index as usize)
            .and_then(|value| value.as_i64())
            .ok_or_else(|| format!("TypeRef {name} is not an integer"))
    };
    let identity = FrozenTypeIdentity {
        high: identity_field("identity_high")?,
        low: identity_field("identity_low")?,
    };
    freeze.category_of(identity)
}

pub(crate) fn build_frozen_type_category_heap_value(
    category: FrozenTypeCategory,
) -> Result<HeapValue, String> {
    let registry = current_registry();
    let schema = registry
        .get("FrozenTypeCategory")
        .ok_or_else(|| "FrozenTypeCategory schema is not registered".to_string())?;
    let variant = schema.variant_id(category.variant_name()).ok_or_else(|| {
        format!(
            "FrozenTypeCategory has no '{}' variant",
            category.variant_name()
        )
    })?;
    typed_slot_into_heap_value(typed_object_for_named_schema(
        "FrozenTypeCategory",
        &[("__variant", KindedSlot::from_int(i64::from(variant)))],
    ))
}

// `pub(super)`-within-`comptime_builtins`: the S3 trait-evidence carriers
// (`trait_evidence.rs`) reuse the SAME slot→heap-value ownership transfer as
// the TypeRef/FrozenTypeCategory carriers — one construction path, no second
// derivation.
pub(super) fn typed_slot_into_heap_value(slot: KindedSlot) -> Result<HeapValue, String> {
    if slot.kind() != NativeKind::Ptr(HeapKind::TypedObject) || slot.raw() == 0 {
        return Err("typed reflection carrier was not a typed object".to_string());
    }
    let ptr = slot.raw() as *const TypedObjectStorage;
    // SAFETY: the kind witness and non-null check above prove this is the live
    // storage pointer owned by `slot`; retain transfers one share to the
    // returned `TypedObjectPtr` before dropping the original slot share.
    unsafe {
        shape_value::v2::refcount::v2_retain(&(*ptr).header);
    }
    drop(slot);
    Ok(HeapValue::TypedObject(TypedObjectPtr::new(ptr)))
}

// E5-deletes: legacy `type_info` string kind vocabulary. Confined to this
// module + the single path-qualified intrinsic caller in the parent module
// (ADR-009 §4.1 "one kind vocabulary"); ticket E5 deletes it. Sentinel:
// `tests::legacy_type_info_vocabulary_is_confined_to_the_legacy_intrinsic_path`.
#[derive(Debug, Clone, Copy)]
enum TypeKindLabel {
    Int,
    Number,
    Bool,
    String,
    Decimal,
    BigInt,
    TypedObject,
    Unit,
    Unresolved,
}

impl TypeKindLabel {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Int => "Int",
            Self::Number => "Number",
            Self::Bool => "Bool",
            Self::String => "String",
            Self::Decimal => "Decimal",
            Self::BigInt => "BigInt",
            Self::TypedObject => "TypedObject",
            Self::Unit => "Unit",
            Self::Unresolved => "Unresolved",
        }
    }
}

/// Legacy `type_info` classification (`TypeKindLabel` string vocabulary).
/// E5 deletes this path; until then it consumes the SAME freeze handle as
/// the typed reflection surface — scoped generic parameters come from the
/// overlay, nominal/alias/enum membership from the freeze's index. No
/// per-site table survives.
// E5-deletes: reachable only from `build_type_info_heap_value` below.
fn classify_legacy_type_info(name: &str, freeze: &FreezeOverlay) -> TypeKindLabel {
    if freeze.is_scoped_parameter(name) {
        return TypeKindLabel::Unresolved;
    }
    let index = freeze.base().index();
    match name {
        "int" | "i64" | "i32" | "i16" | "i8" | "u64" | "u32" | "u16" | "u8" => TypeKindLabel::Int,
        "number" | "f64" | "f32" | "float" => TypeKindLabel::Number,
        "bool" => TypeKindLabel::Bool,
        "string" | "str" => TypeKindLabel::String,
        "decimal" => TypeKindLabel::Decimal,
        "bigint" => TypeKindLabel::BigInt,
        "()" | "unit" | "void" => TypeKindLabel::Unit,
        _ if index.struct_defs.contains_key(name)
            || index.alias_defs.contains_key(name)
            || index.enum_defs.contains_key(name) =>
        {
            TypeKindLabel::TypedObject
        }
        _ => TypeKindLabel::Unresolved,
    }
}

// E5-deletes: legacy `type_info` record builder (`__ComptimeTypeInfo`
// carrier). `pub(super)` — the parent module's `type_info` intrinsic is the
// ONLY caller (path-qualified, never re-exported); ticket E5 deletes the path
// together with `TypeKindLabel` / `classify_legacy_type_info` and the
// `__ComptimeTypeInfo` schema registration in `builtin_schemas.rs`.
pub(super) fn build_type_info_heap_value(
    type_name: &str,
    freeze: &FreezeOverlay,
) -> Result<HeapValue, String> {
    let label = classify_legacy_type_info(type_name, freeze);
    let field_rows: Vec<(String, String, Vec<comptime_target::FieldAnnotation>)> = freeze
        .base()
        .index()
        .struct_defs
        .get(type_name)
        .map(|fields| {
            fields
                .iter()
                .map(|(name, annotation)| {
                    (
                        name.clone(),
                        comptime_target::type_annotation_to_string(annotation),
                        Vec::new(),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let fields = comptime_target::build_field_descriptor_array(&field_rows)
        .map_err(|error| format!("failed to build type_info fields for '{type_name}': {error}"))?;
    typed_slot_into_heap_value(typed_object_for_named_schema(
        "__ComptimeTypeInfo",
        &[
            ("name", super::nb_str(type_name)),
            ("kind", super::nb_str(label.as_str())),
            ("fields", fields),
            (
                "type_ref",
                comptime_target::build_type_ref_descriptor(type_name, Some(label.as_str())),
            ),
        ],
    ))
}

#[cfg(test)]
mod tests;
