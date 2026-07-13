use super::semantic_freeze::{FreezeOverlay, annotation_has_unresolved_inference_variable};
use crate::compiler::comptime_target;
use sha2::{Digest, Sha256};
use shape_ast::ast::{ObjectTypeField, TypeAnnotation};
pub(crate) use shape_runtime::comptime_reflection::FrozenTypeCategory;
use shape_runtime::comptime_reflection::{FloatWidth, FrozenPrimitive, IntegerWidth};
use shape_runtime::type_schema::builtin_schemas::COMPTIME_FROZEN_TYPE_REF_SCHEMA;
use shape_runtime::type_schema::{current_registry, typed_object_for_named_schema};
use shape_value::heap_value::{HeapKind, HeapValue, TypedObjectPtr, TypedObjectStorage};
use shape_value::{KindedSlot, NativeKind};
use std::collections::HashMap;

/// ADR-009 B1 S2: payload descriptors + heap-value builders for the sealed
/// `FrozenType` sum returned by `reflect()`.
pub(crate) mod payloads;

/// The single primitive synonym-family table (ADR-009 §4.1 canonical-inputs
/// rule): each row carries the family's interned synonyms AND its exact
/// width/domain payload from the sealed `FrozenPrimitive` sub-algebra
/// (Dec 50/94). `rebuild_frozen_type_index` derives both the identity map
/// and the identity→payload map from THIS table — there is deliberately no
/// second name table. `bigint` is the named `SignedInteger(Arbitrary)`
/// decision (unbounded width-domain member, logged in `docs/defections.md`).
const PRIMITIVE_SYNONYM_FAMILIES: &[(&[&str], FrozenPrimitive)] = &[
    (&["unit", "void", "()"], FrozenPrimitive::Unit),
    (&["bool"], FrozenPrimitive::Bool),
    (&["char"], FrozenPrimitive::Char),
    (
        &["int", "i64"],
        FrozenPrimitive::SignedInteger(IntegerWidth::W64),
    ),
    (&["i8"], FrozenPrimitive::SignedInteger(IntegerWidth::W8)),
    (&["i16"], FrozenPrimitive::SignedInteger(IntegerWidth::W16)),
    (&["i32"], FrozenPrimitive::SignedInteger(IntegerWidth::W32)),
    (&["u8"], FrozenPrimitive::UnsignedInteger(IntegerWidth::W8)),
    (
        &["u16"],
        FrozenPrimitive::UnsignedInteger(IntegerWidth::W16),
    ),
    (
        &["u32"],
        FrozenPrimitive::UnsignedInteger(IntegerWidth::W32),
    ),
    (
        &["u64"],
        FrozenPrimitive::UnsignedInteger(IntegerWidth::W64),
    ),
    (
        &["bigint"],
        FrozenPrimitive::SignedInteger(IntegerWidth::Arbitrary),
    ),
    (
        &["number", "f64", "float"],
        FrozenPrimitive::BinaryFloat(FloatWidth::W64),
    ),
    (&["f32"], FrozenPrimitive::BinaryFloat(FloatWidth::W32)),
    (&["decimal"], FrozenPrimitive::Decimal),
    (&["string", "str"], FrozenPrimitive::String),
    (&["null"], FrozenPrimitive::Null),
    (&["undefined"], FrozenPrimitive::Undefined),
];

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
    /// ADR-009 A2 (slice S5): frozen trait names (named freeze input 5,
    /// `BytecodeCompiler::known_traits`). `dyn` bounds and trait
    /// intersections in checked type expressions resolve against this set;
    /// an unknown bound is a named rejection in the unknown-identity family.
    pub(crate) trait_names: std::collections::HashSet<String>,
    /// ADR-009 A2 (slice S5): declared generic arity per user STRUCT name —
    /// the freeze-input projection of `struct_generic_info.type_params`
    /// (part of named freeze input 1). Enum generic arity is NOT recoverable
    /// from the schema registry today, so applied enum heads are arity-
    /// unchecked (surfaced S5 decision — no guessing).
    pub(crate) struct_generic_arities: HashMap<String, usize>,
    pub(crate) frozen_type_ids: HashMap<String, FrozenTypeIdentity>,
    pub(crate) frozen_type_categories: HashMap<FrozenTypeIdentity, FrozenTypeCategory>,
    /// ADR-009 B1 S2: exact width/domain payload per Primitive identity,
    /// derived from [`PRIMITIVE_SYNONYM_FAMILIES`] in the same rebuild that
    /// interns the identities (one source, no second derivation).
    pub(crate) frozen_primitive_payloads: HashMap<FrozenTypeIdentity, FrozenPrimitive>,
    /// ADR-009 A2 (slice S5): identity-keyed declared arity for applicable
    /// nominal heads (builtin table + user structs), built by
    /// `rebuild_frozen_type_index`. Identity-keyed so alias heads inherit
    /// their target's arity transparently (Dec 53).
    pub(crate) generic_arities: HashMap<FrozenTypeIdentity, usize>,
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

    /// ADR-009 B1 S2: the shared query API's payload half at the index
    /// level. Enabled payload categories (Primitive / Never / Erased)
    /// return complete typed descriptors; every non-enabled category is
    /// the named R1 per-category rejection — never a partial descriptor.
    /// The unknown-identity freeze-boundary rejection is unchanged.
    pub(super) fn payload_for_identity(
        &self,
        identity: FrozenTypeIdentity,
    ) -> Result<payloads::FrozenPayloadDescriptor, String> {
        use payloads::FrozenPayloadDescriptor;
        match self.category_for_identity(identity)? {
            FrozenTypeCategory::Primitive => self
                .frozen_primitive_payloads
                .get(&identity)
                .copied()
                .map(FrozenPayloadDescriptor::Primitive)
                .ok_or_else(|| {
                    "internal invariant: a Primitive identity was frozen without its \
                     FrozenPrimitive payload"
                        .to_string()
                }),
            FrozenTypeCategory::Never => Ok(FrozenPayloadDescriptor::Never),
            // `any` is the only reachable erased spelling until A2 lands
            // trait-bound syntax: the bound set is complete AND empty.
            FrozenTypeCategory::Erased => Ok(FrozenPayloadDescriptor::Erased { bounds: Vec::new() }),
            pending => Err(payloads::pending_payload_rejection(pending)),
        }
    }

    pub(super) fn rebuild_frozen_type_index(&mut self) {
        let mut ids = HashMap::new();
        let mut categories = HashMap::new();
        let mut primitive_payloads = HashMap::new();
        let mut arities: HashMap<FrozenTypeIdentity, usize> = HashMap::new();

        for (names, primitive) in PRIMITIVE_SYNONYM_FAMILIES {
            let identity = intern_synonyms(
                &mut ids,
                &mut categories,
                names,
                FrozenTypeCategory::Primitive,
            );
            primitive_payloads.insert(identity, *primitive);
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

        // Builtin nominal constructors: one table carries name AND declared
        // arity (S5 R5 — arity is a freeze fact, enforced by the single
        // canonicalizer, identity-keyed so alias heads inherit it).
        for (name, arity) in [
            ("Array", 1),
            ("Vec", 1),
            ("HashMap", 2),
            ("Option", 1),
            ("Result", 2),
            ("Future", 1),
            ("Set", 1),
            ("Deque", 1),
            ("PriorityQueue", 1),
            ("Mutex", 1),
            ("Slice", 1),
        ] {
            let identity = intern_identity(
                &mut ids,
                &mut categories,
                name,
                &format!("nominal:{name}"),
                FrozenTypeCategory::Nominal,
            );
            arities.insert(identity, arity);
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
            let identity = intern_identity(
                &mut ids,
                &mut categories,
                &name,
                &format!("nominal:{name}"),
                FrozenTypeCategory::Nominal,
            );
            // User-struct arity from the declared type parameters (freeze
            // input 1 projection). Enums have no entry — arity-unchecked.
            if let Some(arity) = self.struct_generic_arities.get(&name) {
                arities.insert(identity, *arity);
            }
        }

        // Scoped generic parameters are NOT interned here: they enter through
        // a `FreezeOverlay` (`parameter:{owner}:{name}` identities layered
        // over the shared base), never through the base index (ADR-009 §4.1).

        // Aliases are transparent: an alias receives the exact identity of its
        // canonical target. Iterate to a fixed point so alias chains normalize.
        //
        // ADR-009 A2 (slice S1): composite alias targets (`type Pair =
        // [int, string]`, `type Ids = Array<UserId>`) intern via the SAME
        // canonicalizer as every other composite form, resolving leaves
        // against the module-scope table built so far (a `FreezeOverlay`
        // cannot exist mid-freeze; this table is exactly what the module
        // overlay will read). A target whose leaves are not yet resolvable
        // this round is retried next round; a target that never resolves
        // (unknown name, self-cycle) simply stays un-interned — the later
        // `type_ref` use rejects with the named unknown-identity diagnostic.
        // Termination bound: `aliases.len()` rounds resolve any acyclic
        // chain; interned values are write-once so composite embeddings
        // never re-hash.
        let mut aliases: Vec<_> = self.alias_defs.iter().collect();
        aliases.sort_by(|(left, _), (right, _)| left.cmp(right));
        for _ in 0..=aliases.len() {
            let mut changed = false;
            for (alias, target) in &aliases {
                if let Some(target_name) = target.as_simple_name() {
                    let Some(identity) = ids.get(target_name).copied() else {
                        continue;
                    };
                    changed |= ids.insert((*alias).clone(), identity) != Some(identity);
                    continue;
                }
                let canonical = {
                    let resolve = |name: &str| {
                        let identity = ids.get(name).copied()?;
                        let category = categories.get(&identity).copied()?;
                        Some((identity, category))
                    };
                    let is_trait = |name: &str| self.trait_names.contains(name);
                    let applied_arity =
                        |identity: FrozenTypeIdentity| arities.get(&identity).copied();
                    canonicalize_with(
                        target,
                        &LeafScope {
                            resolve: &resolve,
                            is_trait: &is_trait,
                            applied_arity: &applied_arity,
                        },
                    )
                };
                let Ok(canonical) = canonical else {
                    continue;
                };
                if let Some(previous) = categories.insert(canonical.identity, canonical.category) {
                    assert_eq!(
                        previous, canonical.category,
                        "canonical type identity collision across semantic categories"
                    );
                }
                changed |= ids.insert((*alias).clone(), canonical.identity)
                    != Some(canonical.identity);
            }
            if !changed {
                break;
            }
        }

        self.frozen_type_ids = ids;
        self.frozen_type_categories = categories;
        self.frozen_primitive_payloads = primitive_payloads;
        self.generic_arities = arities;
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
) -> FrozenTypeIdentity {
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
    identity
}

/// ADR-009 A2 (slice S1): canonicalization result for one resolved type
/// expression — the canonical descriptor string, its exhaustive semantic
/// category, and the identity hashed from the descriptor.
///
/// Compile-time-only value confined to `comptime_builtins` (`pub(super)`):
/// it never escapes as a runtime carrier; public `TypeRef` values carry only
/// the 128-bit identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CanonicalType {
    pub(super) descriptor: String,
    pub(super) category: FrozenTypeCategory,
    pub(super) identity: FrozenTypeIdentity,
}

/// The 32-lowercase-hex embedding form of a frozen identity. Every composite
/// descriptor embeds its children in this form (see the descriptor grammar on
/// [`canonicalize_type_annotation`]).
pub(super) fn identity_hex(identity: FrozenTypeIdentity) -> String {
    format!("{:016x}{:016x}", identity.high as u64, identity.low as u64)
}

/// ADR-009 A2 (slice S1): THE single canonicalizer from a resolved
/// `TypeAnnotation` to `(descriptor, category, identity)` (spec §4.1 —
/// no second derivation of any semantic fact).
///
/// Leaves resolve ONLY through the freeze/overlay query API
/// ([`FreezeOverlay::identity_of`] / [`FreezeOverlay::category_of`]): alias
/// transparency, primitive-synonym coalescing and `parameter:{owner}:{name}`
/// scoping are inherited from the freeze, never re-implemented here.
/// Failure is always a named error — never `FrozenTypeIdentity::INVALID`,
/// never a partially populated descriptor.
///
/// # Canonical descriptor grammar (B4/B7 ABI substrate — identity-stable)
///
/// These strings are SHA-256 pre-images of [`FrozenTypeIdentity`] values
/// shared by VM and JIT; changing any rule re-hashes identities (ABI break).
///
/// * **Child embedding** — every child (leaf or composite) embeds as the
///   32-lowercase-hex of its identity (`identity_hex`), written `h` below.
///   Leaf pre-images live in the freeze index (`Primitive:int`,
///   `nominal:Point`, `parameter:{owner}:{name}`, …) and are unchanged.
/// * **Tuple** — `tuple:[h,h,…]`; member order significant.
/// * **Record** — `record:{name:h,name?:h,…}`; fields sorted by byte order
///   of the field NAME (declaration-order independent); `?` marks an
///   optional field; duplicate field names are a named rejection.
/// * **Callable** — `callable:(h,h?,…)->h`; positional parameter order
///   significant; `?` marks an optional parameter; names insignificant.
/// * **Reference** — `reference:&h` / `reference:&mut h`; mutability is
///   descriptor-significant.
/// * **Union** — `union:(h|h|…)`; members deduped and byte-sorted by their
///   hex embedding (source order/duplication insignificant); a union whose
///   members all coalesce to one identity IS that member (no singleton
///   union descriptor exists).
/// * **Erased** — bare `any` resolves as the frozen erased leaf; trait
///   objects are `erased:dyn A+B` with the bound set sorted + deduped by
///   source path name (traits carry no frozen identity — Dec 50/94 erases
///   them to a bound set). S5: every bound must be a trait frozen in this
///   compilation unit (named freeze input, `known_traits`); an unknown
///   bound is a named rejection in the unknown-identity family.
/// * **Applied generic** — `applied:h<h,h,…>`; the head must resolve to a
///   `Nominal` leaf (applying arguments to primitives or parameters is a
///   named rejection); category is `Nominal` (Dec 50/94: applied builtin and
///   user types are nominal with typed arguments); distinct from the bare
///   nominal head. `T[]` / `Array<T>` spellings share one identity. S5: the
///   head's declared arity (builtin table + user-struct type parameters;
///   identity-keyed, alias-transparent) is enforced — a mismatch is a named
///   rejection; enum heads carry no recoverable arity and are unchecked.
/// * **Intersection** — all-object intersections flatten to the Record form
///   above (field collisions are named rejections); all-trait intersections
///   erase to the SAME `erased:dyn` bound-set descriptor as the `dyn`
///   spelling; every other intersection is a named rejection (Dec 50/94
///   rule 3).
/// * **Structural leaves** — `void`/`()`/`never`/`null`/`undefined` resolve
///   through the freeze's synonym table like any other leaf.
///
/// # Rejections
///
/// * Unresolved leaf name at any depth → named error in the
///   `unknown semantic type identity` family, naming the leaf (Dec 48/52).
/// * Inference hole at any depth (analyzer tyvar marker, detected by the
///   ONE freeze-boundary predicate
///   `annotation_has_unresolved_inference_variable`) → named error in the
///   Dec 52 `cannot be frozen … unresolved inference variable` family.
pub(super) fn canonicalize_type_annotation(
    annotation: &TypeAnnotation,
    overlay: &FreezeOverlay,
) -> Result<CanonicalType, String> {
    let resolve = |name: &str| {
        let identity = overlay.identity_of(name)?;
        let category = overlay.category_of(identity).ok()?;
        Some((identity, category))
    };
    let index = overlay.base().index();
    let is_trait = |name: &str| index.trait_names.contains(name);
    let applied_arity = |identity: FrozenTypeIdentity| index.generic_arities.get(&identity).copied();
    canonicalize_with(
        annotation,
        &LeafScope {
            resolve: &resolve,
            is_trait: &is_trait,
            applied_arity: &applied_arity,
        },
    )
}

/// Leaf resolution context for the canonicalizer. Two contexts exist, both
/// projections of the ONE freeze table: the public entry point resolves
/// through a [`FreezeOverlay`], and the alias fixpoint inside
/// [`FrozenTypeIndex::rebuild_frozen_type_index`] resolves through the
/// module-scope base table it is constructing (an overlay cannot exist
/// mid-freeze). Same code path, no second derivation.
///
/// ADR-009 A2 (slice S5): the scope carries THREE projections of the one
/// freeze table — leaf identity/category, the frozen trait-name set (`dyn`
/// bounds / trait intersections), and the identity-keyed declared arity of
/// applicable nominal heads.
struct LeafScope<'a> {
    resolve: &'a dyn Fn(&str) -> Option<(FrozenTypeIdentity, FrozenTypeCategory)>,
    is_trait: &'a dyn Fn(&str) -> bool,
    applied_arity: &'a dyn Fn(FrozenTypeIdentity) -> Option<usize>,
}

fn canonicalize_with(
    annotation: &TypeAnnotation,
    scope: &LeafScope<'_>,
) -> Result<CanonicalType, String> {
    // Dec 52 freeze boundary: an inference hole anywhere in the expression is
    // a named rejection before any descriptor is formed. The predicate is the
    // same exhaustive walk the freeze barrier uses.
    if annotation_has_unresolved_inference_variable(annotation) {
        return Err(
            "semantic freeze rejected: this type expression cannot be frozen because \
             it contains an unresolved inference variable"
                .to_string(),
        );
    }
    canonicalize_resolved(annotation, scope)
}

fn canonicalize_resolved(
    annotation: &TypeAnnotation,
    scope: &LeafScope<'_>,
) -> Result<CanonicalType, String> {
    match annotation {
        TypeAnnotation::Basic(name) => canonical_leaf(name, scope),
        TypeAnnotation::Reference(path) => canonical_leaf(path.as_str(), scope),
        TypeAnnotation::Void => canonical_leaf("void", scope),
        TypeAnnotation::Never => canonical_leaf("never", scope),
        TypeAnnotation::Null => canonical_leaf("null", scope),
        TypeAnnotation::Undefined => canonical_leaf("undefined", scope),
        TypeAnnotation::Array(inner) => {
            canonical_applied("Array", std::slice::from_ref(inner), scope)
        }
        TypeAnnotation::Generic { name, args } if args.is_empty() => {
            canonical_leaf(name.as_str(), scope)
        }
        TypeAnnotation::Generic { name, args } => canonical_applied(name.as_str(), args, scope),
        TypeAnnotation::Tuple(items) => {
            let mut embedded = Vec::with_capacity(items.len());
            for item in items {
                embedded.push(identity_hex(canonicalize_resolved(item, scope)?.identity));
            }
            Ok(composite(
                format!("tuple:[{}]", embedded.join(",")),
                FrozenTypeCategory::Tuple,
            ))
        }
        TypeAnnotation::Object(fields) => canonical_record(fields, scope),
        TypeAnnotation::Function { params, returns } => {
            let mut embedded = Vec::with_capacity(params.len());
            for param in params {
                let member = canonicalize_resolved(&param.type_annotation, scope)?;
                embedded.push(format!(
                    "{}{}",
                    identity_hex(member.identity),
                    if param.optional { "?" } else { "" }
                ));
            }
            let returns = canonicalize_resolved(returns, scope)?;
            Ok(composite(
                format!(
                    "callable:({})->{}",
                    embedded.join(","),
                    identity_hex(returns.identity)
                ),
                FrozenTypeCategory::Callable,
            ))
        }
        TypeAnnotation::Borrow { mutable, inner } => {
            let member = canonicalize_resolved(inner, scope)?;
            Ok(composite(
                format!(
                    "reference:&{}{}",
                    if *mutable { "mut " } else { "" },
                    identity_hex(member.identity)
                ),
                FrozenTypeCategory::Reference,
            ))
        }
        TypeAnnotation::Union(items) => {
            if items.is_empty() {
                return Err("type_ref union type must name at least one member".to_string());
            }
            let mut members = Vec::with_capacity(items.len());
            for item in items {
                members.push(canonicalize_resolved(item, scope)?);
            }
            let mut embedded: Vec<String> = members
                .iter()
                .map(|member| identity_hex(member.identity))
                .collect();
            embedded.sort();
            embedded.dedup();
            if embedded.len() == 1 {
                // All members coalesce to one identity: the union IS its
                // member (int | i64 == int); no singleton union descriptor.
                return Ok(members.into_iter().next().expect("non-empty union"));
            }
            Ok(composite(
                format!("union:({})", embedded.join("|")),
                FrozenTypeCategory::Union,
            ))
        }
        TypeAnnotation::Intersection(items) => {
            // Dec 50/94 rule 3, one classification rule (S5 R8): an
            // intersection whose members are ALL structural object types
            // normalizes to a Record; one whose members are ALL frozen trait
            // names erases to the same bound-set descriptor as the `dyn`
            // spelling; anything else is a named rejection.
            if items
                .iter()
                .all(|item| matches!(item, TypeAnnotation::Object(_)))
            {
                let mut merged: Vec<ObjectTypeField> = Vec::new();
                for item in items {
                    let TypeAnnotation::Object(fields) = item else {
                        unreachable!("all members checked as object types");
                    };
                    merged.extend(fields.iter().cloned());
                }
                return canonical_record(&merged, scope);
            }
            let mut bound_names = Vec::with_capacity(items.len());
            for item in items {
                let name = match item {
                    TypeAnnotation::Basic(name) => Some(name.clone()),
                    TypeAnnotation::Reference(path) => Some(path.to_string()),
                    _ => None,
                };
                match name {
                    Some(name) if (scope.is_trait)(&name) => bound_names.push(name),
                    _ => {
                        return Err(format!(
                            "type_ref cannot canonicalize this intersection: members must \
                             be either all structural object types (normalizing to a \
                             record) or all trait bounds (erasing to a bound set) per \
                             Dec 50/94; member '{}' is neither",
                            item.to_type_string()
                        ));
                    }
                }
            }
            canonical_erased_bounds(bound_names)
        }
        TypeAnnotation::Dyn(bounds) => {
            if bounds.is_empty() {
                return Err(
                    "type_ref erased dyn type must name at least one trait bound".to_string(),
                );
            }
            // S5 R2 (dyn case): bounds resolve against the frozen trait-name
            // set — an unknown bound is a named rejection in the
            // unknown-identity family, naming the bound.
            let mut names = Vec::with_capacity(bounds.len());
            for path in bounds {
                let name = path.to_string();
                if !(scope.is_trait)(&name) {
                    return Err(format!(
                        "type_ref received an unknown semantic type identity: trait bound \
                         '{name}' is not a trait frozen in this compilation unit"
                    ));
                }
                names.push(name);
            }
            canonical_erased_bounds(names)
        }
    }
}

/// Shared erased-bound-set constructor (Dec 50/94): `dyn A + B` and the
/// trait-intersection spelling `A + B` reach ONE descriptor — bound names
/// sorted + deduped, so the bound set is source-order independent.
fn canonical_erased_bounds(mut names: Vec<String>) -> Result<CanonicalType, String> {
    names.sort();
    names.dedup();
    Ok(composite(
        format!("erased:dyn {}", names.join("+")),
        FrozenTypeCategory::Erased,
    ))
}

fn composite(descriptor: String, category: FrozenTypeCategory) -> CanonicalType {
    let identity = FrozenTypeIdentity::from_canonical_descriptor(&descriptor);
    CanonicalType {
        descriptor,
        category,
        identity,
    }
}

fn canonical_leaf(name: &str, scope: &LeafScope<'_>) -> Result<CanonicalType, String> {
    let Some((identity, category)) = (scope.resolve)(name) else {
        return Err(format!(
            "type_ref received an unknown semantic type identity: type name '{name}' \
             is not frozen in this compilation unit"
        ));
    };
    Ok(CanonicalType {
        descriptor: identity_hex(identity),
        category,
        identity,
    })
}

fn canonical_record(
    fields: &[ObjectTypeField],
    scope: &LeafScope<'_>,
) -> Result<CanonicalType, String> {
    let mut entries = Vec::with_capacity(fields.len());
    for field in fields {
        let member = canonicalize_resolved(&field.type_annotation, scope)?;
        entries.push((
            field.name.as_str(),
            field.optional,
            identity_hex(member.identity),
        ));
    }
    // Field-name byte sort: record identity is declaration-order independent.
    entries.sort_by(|left, right| left.0.cmp(right.0));
    for window in entries.windows(2) {
        if window[0].0 == window[1].0 {
            return Err(format!(
                "type_ref record type declares duplicate field '{}'",
                window[0].0
            ));
        }
    }
    let rendered: Vec<String> = entries
        .iter()
        .map(|(name, optional, hex)| {
            format!("{name}{}:{hex}", if *optional { "?" } else { "" })
        })
        .collect();
    Ok(composite(
        format!("record:{{{}}}", rendered.join(",")),
        FrozenTypeCategory::Record,
    ))
}

fn canonical_applied(
    head: &str,
    args: &[TypeAnnotation],
    scope: &LeafScope<'_>,
) -> Result<CanonicalType, String> {
    let head_leaf = canonical_leaf(head, scope)?;
    if head_leaf.category != FrozenTypeCategory::Nominal {
        return Err(format!(
            "type_ref cannot apply type arguments to '{head}': only nominal type \
             constructors accept type arguments (found category {})",
            head_leaf.category.variant_name()
        ));
    }
    // S5 R5: declared arity is a freeze fact (builtin table + user-struct
    // type parameters), identity-keyed so alias heads inherit it. Heads with
    // no recoverable arity (enums today) are unchecked — surfaced decision,
    // never a guess.
    if let Some(expected) = (scope.applied_arity)(head_leaf.identity)
        && expected != args.len()
    {
        return Err(format!(
            "type_ref applied type '{head}' expects {expected} type argument(s), but {} \
             were provided",
            args.len()
        ));
    }
    let mut embedded = Vec::with_capacity(args.len());
    for arg in args {
        embedded.push(identity_hex(canonicalize_resolved(arg, scope)?.identity));
    }
    Ok(composite(
        format!(
            "applied:{}<{}>",
            identity_hex(head_leaf.identity),
            embedded.join(",")
        ),
        FrozenTypeCategory::Nominal,
    ))
}

pub(crate) fn build_frozen_type_ref_heap_value(
    identity: FrozenTypeIdentity,
    freeze: &FreezeOverlay,
) -> Result<HeapValue, String> {
    freeze.category_of(identity)?;
    typed_slot_into_heap_value(typed_object_for_named_schema(
        COMPTIME_FROZEN_TYPE_REF_SCHEMA,
        &[
            ("identity_high", KindedSlot::from_int(identity.high)),
            ("identity_low", KindedSlot::from_int(identity.low)),
        ],
    ))
}

/// Read the frozen semantic identity out of an opaque `TypeRef` argument
/// slot. The ONE TypeRef-argument reader shared by every TypeRef-consuming
/// intrinsic (`type_category`, `reflect` — ADR-009 B1 S3); `caller` names
/// the intrinsic in each R4 diagnostic ("<caller> expects a TypeRef value"
/// family), so both intrinsics reject malformed arguments identically.
fn frozen_identity_from_ref(
    slot: &KindedSlot,
    caller: &str,
) -> Result<FrozenTypeIdentity, String> {
    if slot.kind() != NativeKind::Ptr(HeapKind::TypedObject) {
        return Err(format!("{caller} expects a TypeRef value"));
    }
    let storage = slot
        .as_typed_object_storage()
        .ok_or_else(|| format!("{caller} received a null TypeRef value"))?;
    let schema = shape_runtime::type_schema::lookup_schema_by_id_public(storage.schema_id as u32)
        .ok_or_else(|| {
        format!(
            "{caller} could not resolve TypeRef schema id {}",
            storage.schema_id
        )
    })?;
    if schema.name != COMPTIME_FROZEN_TYPE_REF_SCHEMA {
        return Err(format!("{caller} expects TypeRef, got '{}'", schema.name));
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
    Ok(FrozenTypeIdentity {
        high: identity_field("identity_high")?,
        low: identity_field("identity_low")?,
    })
}

pub(crate) fn frozen_type_category_from_ref(
    slot: &KindedSlot,
    freeze: &FreezeOverlay,
) -> Result<FrozenTypeCategory, String> {
    let identity = frozen_identity_from_ref(slot, "type_category")?;
    freeze.category_of(identity)
}

/// ADR-009 B1 S3: `reflect(TypeRef<T>) -> FrozenType<T>` — identity from
/// the TypeRef argument (same reader as `type_category`, reflect-named R4
/// diagnostics), payload from the ONE freeze query API (`payload_of`),
/// carrier from the S2 payload builders. R1 per-category rejections and
/// the unknown-identity freeze-boundary rejection propagate unchanged.
pub(crate) fn frozen_type_from_ref(
    slot: &KindedSlot,
    freeze: &FreezeOverlay,
) -> Result<HeapValue, String> {
    let identity = frozen_identity_from_ref(slot, "reflect")?;
    payloads::build_frozen_type_heap_value(identity, freeze)
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
