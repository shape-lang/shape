use crate::compiler::comptime_target;
use sha2::{Digest, Sha256};
use shape_ast::ast::TypeAnnotation;
pub(crate) use shape_runtime::comptime_reflection::FrozenTypeCategory;
use shape_runtime::type_schema::builtin_schemas::COMPTIME_FROZEN_TYPE_REF_SCHEMA;
use shape_runtime::type_schema::{current_registry, typed_object_for_named_schema};
use shape_value::heap_value::{HeapKind, HeapValue, TypedObjectPtr, TypedObjectStorage};
use shape_value::{KindedSlot, NativeKind};
use std::collections::{HashMap, HashSet};

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

    fn from_canonical_descriptor(descriptor: &str) -> Self {
        let digest = Sha256::digest(descriptor.as_bytes());
        let high = i64::from_be_bytes(digest[0..8].try_into().expect("8-byte hash prefix"));
        let low = i64::from_be_bytes(digest[8..16].try_into().expect("8-byte hash suffix"));
        Self { high, low }
    }
}

/// Immutable semantic type table handed from the outer compiler to one
/// comptime mini-VM. Public `TypeRef` values carry only a canonical semantic
/// fingerprint, never a rendered type name or snapshot-local ordinal.
#[derive(Debug, Clone, Default)]
pub(crate) struct TypeReflectionSnapshot {
    pub(crate) struct_defs: HashMap<String, Vec<(String, TypeAnnotation)>>,
    pub(crate) enum_defs: HashMap<String, Vec<String>>,
    pub(crate) alias_defs: HashMap<String, TypeAnnotation>,
    pub(crate) known_type_params: HashSet<String>,
    parameter_owner: Option<String>,
    frozen_type_ids: HashMap<String, FrozenTypeIdentity>,
    frozen_type_categories: HashMap<FrozenTypeIdentity, FrozenTypeCategory>,
}

impl TypeReflectionSnapshot {
    pub(crate) fn frozen_type_id(&self, name: &str) -> Option<FrozenTypeIdentity> {
        self.frozen_type_ids.get(name).copied()
    }

    fn category_for_identity(
        &self,
        identity: FrozenTypeIdentity,
    ) -> Result<FrozenTypeCategory, String> {
        self.frozen_type_categories
            .get(&identity)
            .copied()
            .ok_or_else(|| "type_ref received an unknown semantic type identity".to_string())
    }

    fn rebuild_frozen_type_index(&mut self) {
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

        let mut parameters: Vec<_> = self.known_type_params.iter().cloned().collect();
        parameters.sort();
        let parameter_owner = self.parameter_owner.as_deref().unwrap_or("<module>");
        for name in parameters {
            intern_identity(
                &mut ids,
                &mut categories,
                &name,
                &format!("parameter:{parameter_owner}:{name}"),
                FrozenTypeCategory::Parameter,
            );
        }

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

pub(crate) fn build_type_reflection_snapshot(
    compiler: &crate::compiler::BytecodeCompiler,
    enclosing_type_params: &[String],
) -> TypeReflectionSnapshot {
    let mut snapshot = TypeReflectionSnapshot::default();
    for (name, (field_names, _span)) in &compiler.struct_types {
        let field_types = compiler
            .struct_generic_info
            .get(name)
            .map(|info| info.runtime_field_types.clone())
            .unwrap_or_default();
        let ordered = field_names
            .iter()
            .filter_map(|field_name| {
                field_types
                    .get(field_name)
                    .cloned()
                    .map(|annotation| (field_name.clone(), annotation))
            })
            .collect();
        snapshot.struct_defs.insert(name.clone(), ordered);
    }
    for (alias, target) in &compiler.type_aliases {
        snapshot
            .alias_defs
            .insert(alias.clone(), TypeAnnotation::Basic(target.clone()));
    }
    for type_name in compiler
        .type_tracker
        .schema_registry()
        .type_names()
        .map(str::to_string)
        .collect::<Vec<_>>()
    {
        let Some(schema) = compiler.type_tracker.schema_registry().get(&type_name) else {
            continue;
        };
        let Some(enum_info) = schema.get_enum_info() else {
            continue;
        };
        snapshot.enum_defs.insert(
            type_name,
            enum_info
                .variants
                .iter()
                .map(|variant| variant.name.clone())
                .collect(),
        );
    }
    snapshot
        .known_type_params
        .extend(enclosing_type_params.iter().cloned());
    if let Some(function) = compiler
        .current_function
        .and_then(|index| compiler.program.functions.get(index))
        && let Some(definition) = compiler.function_defs.get(&function.name)
    {
        snapshot.parameter_owner = Some(function.name.clone());
        if let Some(parameters) = &definition.type_params {
            snapshot.known_type_params.extend(
                parameters
                    .iter()
                    .map(|parameter| parameter.name().to_string()),
            );
        }
    }
    snapshot.rebuild_frozen_type_index();
    snapshot
}

pub(crate) fn build_frozen_type_ref_heap_value(
    identity: FrozenTypeIdentity,
    snapshot: &TypeReflectionSnapshot,
) -> Result<HeapValue, String> {
    snapshot.category_for_identity(identity)?;
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
    snapshot: &TypeReflectionSnapshot,
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
    snapshot.category_for_identity(identity)
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

fn typed_slot_into_heap_value(slot: KindedSlot) -> Result<HeapValue, String> {
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

fn classify_legacy_type_info(name: &str, snapshot: &TypeReflectionSnapshot) -> TypeKindLabel {
    if snapshot.known_type_params.contains(name) {
        return TypeKindLabel::Unresolved;
    }
    match name {
        "int" | "i64" | "i32" | "i16" | "i8" | "u64" | "u32" | "u16" | "u8" => TypeKindLabel::Int,
        "number" | "f64" | "f32" | "float" => TypeKindLabel::Number,
        "bool" => TypeKindLabel::Bool,
        "string" | "str" => TypeKindLabel::String,
        "decimal" => TypeKindLabel::Decimal,
        "bigint" => TypeKindLabel::BigInt,
        "()" | "unit" | "void" => TypeKindLabel::Unit,
        _ if snapshot.struct_defs.contains_key(name)
            || snapshot.alias_defs.contains_key(name)
            || snapshot.enum_defs.contains_key(name) =>
        {
            TypeKindLabel::TypedObject
        }
        _ => TypeKindLabel::Unresolved,
    }
}

pub(crate) fn build_type_info_heap_value(
    type_name: &str,
    snapshot: &TypeReflectionSnapshot,
) -> Result<HeapValue, String> {
    let label = classify_legacy_type_info(type_name, snapshot);
    let field_rows: Vec<(String, String, Vec<comptime_target::FieldAnnotation>)> = snapshot
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
