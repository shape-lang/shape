use super::semantic_freeze::{FreezeOverlay, annotation_has_unresolved_inference_variable};
use crate::compiler::comptime_target;
use sha2::{Digest, Sha256};
use shape_ast::ast::{ObjectTypeField, TypeAnnotation};
pub(crate) use shape_runtime::comptime_reflection::FrozenTypeCategory;
use shape_runtime::comptime_reflection::{FloatWidth, FrozenPrimitive, IntegerWidth, ParamKind};
use shape_runtime::type_schema::TypeSchema;
use shape_runtime::type_schema::builtin_schemas::{
    COMPTIME_APPLIED_TYPE_SCHEMA, COMPTIME_FROZEN_TYPE_CONSTRUCTOR_REF_SCHEMA,
    COMPTIME_FROZEN_TYPE_REF_SCHEMA,
};
use shape_runtime::type_schema::{current_registry, typed_object_for_named_schema};
use shape_value::heap_value::{HeapKind, HeapValue, TypedObjectPtr, TypedObjectStorage};
use shape_value::v2::typed_array::{ELEM_TYPE_I64, TypedArray, stamp_elem_type};
use shape_value::{KindedSlot, NativeKind, ValueSlot};
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
    /// ADR-009 A2 (slice S5): frozen trait names (named freeze input 5,
    /// `BytecodeCompiler::known_traits`). `dyn` bounds and trait
    /// intersections in checked type expressions resolve against this set;
    /// an unknown bound is a named rejection in the unknown-identity family.
    pub(crate) trait_names: std::collections::HashSet<String>,
    /// ADR-009 A2 (slice S5) + B4 (Dec 54): declared ordered generic
    /// PARAMETER KINDS per user STRUCT name — the freeze-input projection of
    /// `struct_generic_info.type_params` (each `TypeParam` mapped to its
    /// [`ParamKind`], part of named freeze input 1). Arity is the vector
    /// length: ONE projection, not a separate arity table. Enum generic
    /// arity/kinds are NOT recoverable from the schema registry today, so
    /// applied enum heads are arity/kind-unchecked in the A2 spelling path
    /// and a named surface-and-stop under the B4 `param_kinds_of` query
    /// (no guessing).
    pub(crate) struct_generic_param_kinds: HashMap<String, Vec<ParamKind>>,
    pub(crate) frozen_type_ids: HashMap<String, FrozenTypeIdentity>,
    pub(crate) frozen_type_categories: HashMap<FrozenTypeIdentity, FrozenTypeCategory>,
    /// ADR-009 B1 S2: exact width/domain payload per Primitive identity,
    /// derived from [`PRIMITIVE_SYNONYM_FAMILIES`] in the same rebuild that
    /// interns the identities (one source, no second derivation).
    pub(crate) frozen_primitive_payloads: HashMap<FrozenTypeIdentity, FrozenPrimitive>,
    /// ADR-009 A2 (slice S5) + B4 (Dec 54): identity-keyed declared ordered
    /// generic PARAMETER KINDS for applicable nominal heads (builtin table +
    /// user structs), built by `rebuild_frozen_type_index`. Arity is the
    /// vector length — the SINGLE arity/kind source (spec: no second table).
    /// Identity-keyed so alias heads inherit their target's kinds transparently
    /// (Dec 53).
    pub(crate) generic_param_kinds: HashMap<FrozenTypeIdentity, Vec<ParamKind>>,
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
            FrozenTypeCategory::Erased => {
                // The base-frozen `any` leaf carries the complete AND empty
                // bound set. Every OTHER Erased identity the base can hold
                // is an alias-fixpoint-interned `erased:dyn …` bound set
                // (A2): its typed bound elements land with ticket B2
                // (`FrozenErasedBound` is uninhabited until then), so it is
                // the named bounded-erased rejection — never an empty
                // (partial) bound set.
                if self.frozen_type_id("any") == Some(identity) {
                    Ok(FrozenPayloadDescriptor::Erased { bounds: Vec::new() })
                } else {
                    Err(payloads::bounded_erased_payload_rejection())
                }
            }
            pending => Err(payloads::pending_payload_rejection(pending)),
        }
    }

    pub(super) fn rebuild_frozen_type_index(&mut self) {
        let mut ids = HashMap::new();
        let mut categories = HashMap::new();
        let mut primitive_payloads = HashMap::new();
        let mut param_kinds: HashMap<FrozenTypeIdentity, Vec<ParamKind>> = HashMap::new();

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
        // canonicalizer, identity-keyed so alias heads inherit it). Every
        // builtin generic parameter is a TYPE parameter (ADR-009 B4, Dec 54):
        // no builtin declares a const generic, so arity `n` projects to
        // `[ParamKind::Type; n]` — arity is the vector length.
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
            param_kinds.insert(identity, vec![ParamKind::Type; arity]);
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
            // User-struct param kinds from the declared type parameters
            // (freeze input 1 projection; arity = vector length). Enums have
            // no entry — arity/kind-unchecked (B4 `param_kinds_of` surfaces).
            if let Some(kinds) = self.struct_generic_param_kinds.get(&name) {
                param_kinds.insert(identity, kinds.clone());
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
                        |identity: FrozenTypeIdentity| param_kinds.get(&identity).map(Vec::len);
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
                changed |=
                    ids.insert((*alias).clone(), canonical.identity) != Some(canonical.identity);
            }
            if !changed {
                break;
            }
        }

        self.frozen_type_ids = ids;
        self.frozen_type_categories = categories;
        self.frozen_primitive_payloads = primitive_payloads;
        self.generic_param_kinds = param_kinds;
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
/// * **Union** — `union:(h|h|…)`; membership is an associative SET: a
///   syntactically nested union (the parenthesized spelling the grammar
///   admits, `(int | string) | bool`) splices its members into the
///   enclosing union, then members dedup and byte-sort by their hex
///   embedding (source order/duplication/grouping insignificant); a union
///   whose members all coalesce to one identity IS that member (no
///   singleton union descriptor exists).
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
    let applied_arity =
        |identity: FrozenTypeIdentity| index.generic_param_kinds.get(&identity).map(Vec::len);
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
            // Union membership is an associative set (descriptor grammar
            // above): a syntactically nested union splices its members into
            // the enclosing union BEFORE dedup/byte-sort, so
            // `(int | string) | bool` and `int | string | bool` mint one
            // identity and `int | (int | string)` cannot escape member
            // dedup. Descriptors are the B4/B7 ABI substrate — an opaque
            // nested-union embedding would fork semantically equal unions
            // into distinct identities.
            let mut flattened = Vec::with_capacity(items.len());
            flatten_union_members(items, &mut flattened);
            if flattened.is_empty() {
                return Err("type_ref union type must name at least one member".to_string());
            }
            let mut members = Vec::with_capacity(flattened.len());
            for item in flattened {
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

/// Union members splice associatively (set semantics — see the descriptor
/// grammar on [`canonicalize_type_annotation`]): a syntactically nested
/// union contributes its members to the enclosing union, never an opaque
/// child identity. Purely structural — leaves (including alias names) are
/// untouched and still resolve through the one freeze query API.
fn flatten_union_members<'a>(items: &'a [TypeAnnotation], out: &mut Vec<&'a TypeAnnotation>) {
    for item in items {
        match item {
            TypeAnnotation::Union(nested) => flatten_union_members(nested, out),
            other => out.push(other),
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
        .map(|(name, optional, hex)| format!("{name}{}:{hex}", if *optional { "?" } else { "" }))
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

// ============================================================================
// ADR-009 B4 (Stage 2, Dec 54): uniform nominal application — constructor
// descriptors + apply/refine/type_argument canonicalizers over the SAME
// frozen identities A2 mints. One model for zero-arg nominals, builtins, user
// generics, and const-generic applications; no per-type reflection variant.
//
// Identity-equality is the load-bearing invariant: `canonical_apply` over a
// `Type` argument reproduces `canonical_applied`'s EXACT
// `applied:<head_hex><arg_hex,…>` descriptor, so
// `identity(apply(constructor(Option), [int]))` equals
// `identity(type_ref(Option<int>))` both directions. Const arguments extend
// the SAME uniform hex-embedding grammar (no descriptor fork); they have no
// A2 spelling (the parser rejects untyped const applications) and are checked
// only through this path.
// ============================================================================

/// The compile-time descriptor for a nominal type CONSTRUCTOR
/// (`type_constructor<C>()`). Carries the frozen head identity — the SAME
/// `identity_hex(head)` A2's applied path embeds, so [`canonical_apply`]
/// reproduces the exact `applied:` descriptor — plus its own constructor
/// identity (distinct from the bare nominal leaf and from any application).
///
/// Compile-time-only, confined to `comptime_builtins` (`pub(super)`): it never
/// escapes as a runtime carrier; the public `TypeConstructorRef` transports
/// only the 128-bit head identity halves (S2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ConstructorDescriptor {
    /// The frozen nominal head this constructor applies over.
    pub(super) head_identity: FrozenTypeIdentity,
    /// `constructor:<head_hex>` — the constructor's own canonical descriptor.
    pub(super) descriptor: String,
    /// Hash of [`Self::descriptor`].
    pub(super) identity: FrozenTypeIdentity,
}

/// One checked argument supplied to [`canonical_apply`]. Uniform over BOTH
/// parameter kinds: a `Type` argument is a checked `TypeRef` identity; a
/// `Const` argument is a checked `const_arg` value canonicalized to its own
/// identity. Both contribute their `identity_hex` to the applied descriptor,
/// so a mixed type/const application embeds uniformly as `applied:h<h,h,…>` —
/// no per-kind descriptor grammar fork (spec §3.1: no untyped argument
/// arrays, no partial descriptors).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AppliedArg {
    /// A checked type argument, carrying its frozen `TypeRef` identity.
    Type(FrozenTypeIdentity),
    /// A checked const argument, carrying its canonical const-value identity.
    Const(FrozenTypeIdentity),
}

impl AppliedArg {
    /// The declared parameter kind this argument must be applied against.
    pub(super) fn kind(self) -> ParamKind {
        match self {
            Self::Type(_) => ParamKind::Type,
            Self::Const(_) => ParamKind::Const,
        }
    }

    /// The frozen identity this argument embeds into the applied descriptor.
    pub(super) fn identity(self) -> FrozenTypeIdentity {
        match self {
            Self::Type(identity) | Self::Const(identity) => identity,
        }
    }
}

/// The decomposition of an `applied:` descriptor recovered by
/// [`canonical_refine`]: the frozen head identity and the ordered argument
/// identities. Descriptors embed every argument as a bare `identity_hex`
/// (kind-erased), so refine recovers identities, not kinds — the round-trip
/// contract [`type_argument`] reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RefinedApplication {
    pub(super) head_identity: FrozenTypeIdentity,
    pub(super) arg_identities: Vec<FrozenTypeIdentity>,
}

/// The 32-lowercase-hex inverse of [`identity_hex`]: parse a child-embedding
/// hex back to its [`FrozenTypeIdentity`]. `None` for any string that is not
/// exactly 32 hex digits (so a malformed / truncated descriptor refines to
/// `None`, never a fabricated identity).
pub(super) fn identity_from_hex(hex: &str) -> Option<FrozenTypeIdentity> {
    if hex.len() != 32 {
        return None;
    }
    let high = u64::from_str_radix(hex.get(0..16)?, 16).ok()? as i64;
    let low = u64::from_str_radix(hex.get(16..32)?, 16).ok()? as i64;
    Some(FrozenTypeIdentity { high, low })
}

/// ADR-009 B4: mint a constructor descriptor for the nominal head `head`
/// (`type_constructor<C>()`). The head resolves through the ONE freeze query
/// API; a non-nominal head is the named non-Nominal rejection (the same wall
/// [`canonical_applied`] raises), an unknown name the named unknown-identity
/// rejection. The minted descriptor is distinct from the bare nominal leaf so
/// a `TypeConstructorRef` is never confused with a `TypeRef`.
pub(super) fn canonical_constructor(
    head: &str,
    overlay: &FreezeOverlay,
) -> Result<ConstructorDescriptor, String> {
    let Some(identity) = overlay.identity_of(head) else {
        return Err(format!(
            "type_constructor received an unknown semantic type identity: type name \
             '{head}' is not frozen in this compilation unit"
        ));
    };
    let category = overlay.category_of(identity)?;
    if category != FrozenTypeCategory::Nominal {
        return Err(format!(
            "type_constructor cannot build a constructor for '{head}': only nominal \
             type constructors accept type arguments (found category {})",
            category.variant_name()
        ));
    }
    let descriptor = format!("constructor:{}", identity_hex(identity));
    let constructor_identity = FrozenTypeIdentity::from_canonical_descriptor(&descriptor);
    Ok(ConstructorDescriptor {
        head_identity: identity,
        descriptor,
        identity: constructor_identity,
    })
}

/// ADR-009 B4: mint a checked const argument from an integer value
/// (`const_arg(N)` for a `const N: int` parameter). The value canonicalizes
/// to its own `const:int:{value}` identity so it embeds uniformly in the
/// applied descriptor. Only this checked path produces a const argument — a
/// bare/untyped const literal keeps the parser's R9 rejection.
pub(super) fn canonical_const_arg(value: i64) -> AppliedArg {
    AppliedArg::Const(FrozenTypeIdentity::from_canonical_descriptor(&format!(
        "const:int:{value}"
    )))
}

/// ADR-009 B4: apply checked arguments to a constructor, producing the
/// canonical `TypeRef<Applied>`. Arity and type-vs-const kind checks reuse the
/// ONE param-kind projection through [`FreezeOverlay::param_kinds_of`] — never
/// a re-derived arity/kind table; a generic enum head surfaces-and-stops there
/// (never a guessed kind). A `Type`-argument application reproduces
/// [`canonical_applied`]'s exact descriptor, so its identity equals the A2
/// `type_ref(Head<Args>)` spelling.
pub(super) fn canonical_apply(
    constructor: &ConstructorDescriptor,
    args: &[AppliedArg],
    overlay: &FreezeOverlay,
) -> Result<CanonicalType, String> {
    let kinds = overlay.param_kinds_of(constructor.head_identity)?;
    if kinds.len() != args.len() {
        return Err(format!(
            "apply on type constructor '{}' expects {} type argument(s), but {} were \
             provided",
            constructor_head_name(constructor, overlay),
            kinds.len(),
            args.len()
        ));
    }
    for (position, (declared, supplied)) in kinds.iter().zip(args).enumerate() {
        if *declared != supplied.kind() {
            return Err(format!(
                "apply argument {position} has the wrong kind: parameter is a \
                 {}-parameter but a {}-argument was supplied",
                declared.variant_name(),
                supplied.kind().variant_name()
            ));
        }
    }
    if args.is_empty() {
        // A zero-argument application IS the bare nominal (the A2 spelling
        // `type_ref(Head)` — no `applied:h<>` descriptor exists); this keeps
        // the model uniform for zero-arg nominals, and refine over the result
        // returns None (round-trips only over genuine applications).
        return Ok(CanonicalType {
            descriptor: identity_hex(constructor.head_identity),
            category: FrozenTypeCategory::Nominal,
            identity: constructor.head_identity,
        });
    }
    let embedded: Vec<String> = args
        .iter()
        .map(|arg| identity_hex(arg.identity()))
        .collect();
    Ok(composite(
        format!(
            "applied:{}<{}>",
            identity_hex(constructor.head_identity),
            embedded.join(",")
        ),
        FrozenTypeCategory::Nominal,
    ))
}

/// Best-effort diagnostic name for a constructor head: the frozen type name if
/// one froze to this identity, else the raw hex (a builtin/user nominal always
/// has a name; the hex fallback is defensive).
fn constructor_head_name(constructor: &ConstructorDescriptor, overlay: &FreezeOverlay) -> String {
    overlay
        .type_names_for_identity(constructor.head_identity)
        .first()
        .map(|name| (*name).to_string())
        .unwrap_or_else(|| identity_hex(constructor.head_identity))
}

/// ADR-009 B4: decompose an `applied:` descriptor back to its head identity
/// and ordered argument identities (`nominal.refine(constructor)`). Returns
/// `None` for a bare-nominal / non-applied descriptor — refine round-trips
/// only over genuine applications, never a partial answer, never an error.
pub(super) fn canonical_refine(applied_descriptor: &str) -> Option<RefinedApplication> {
    let body = applied_descriptor.strip_prefix("applied:")?;
    let head_identity = identity_from_hex(body.get(0..32)?)?;
    let inner = body.get(32..)?.strip_prefix('<')?.strip_suffix('>')?;
    let mut arg_identities = Vec::new();
    if !inner.is_empty() {
        for arg_hex in inner.split(',') {
            arg_identities.push(identity_from_hex(arg_hex)?);
        }
    }
    Some(RefinedApplication {
        head_identity,
        arg_identities,
    })
}

/// ADR-009 B4: the frozen identity of the `index`-th argument of a refined
/// application (`applied.type_argument<I>()`). `index` is a checked const
/// index, not a string key; an out-of-range index is a named rejection.
pub(super) fn type_argument(
    applied: &RefinedApplication,
    index: usize,
) -> Result<FrozenTypeIdentity, String> {
    applied.arg_identities.get(index).copied().ok_or_else(|| {
        format!(
            "type_argument index {index} is out of range: this applied type has {} type \
             argument(s)",
            applied.arg_identities.len()
        )
    })
}

// ============================================================================
// ADR-009 B4 (Stage 2, Dec 54) slice S2: runtime carriers + orchestration for
// uniform nominal application. `type_constructor(C)` issues a
// `TypeConstructorRef` carrying the frozen HEAD identity; `.apply(args)` checks
// arity/kind through the ONE freeze param-kind projection and issues an
// `AppliedType` whose identity is EQUAL to the A2 `type_ref(Head<Args>)`
// spelling; `.refine(constructor)` and `.type_argument(i)` read the stored
// head/arg identities directly (no one-way-hash inversion). Every decode is
// schema-name-checked (the TraitRef/ImplRef forgery-blocking precedent); every
// identity crosses as int halves — no strings, no partial descriptors.
// ============================================================================

/// Rebuild the pure [`ConstructorDescriptor`] from a frozen head identity — the
/// runtime inverse of the S1 name-keyed [`canonical_constructor`]. The
/// intrinsic transports only the head identity halves (no name string), so this
/// re-mints the `constructor:<head_hex>` descriptor + its own identity exactly
/// as the name-keyed path does. Kept private: `canonical_apply` is the only
/// consumer.
fn constructor_descriptor_from_head(head_identity: FrozenTypeIdentity) -> ConstructorDescriptor {
    let descriptor = format!("constructor:{}", identity_hex(head_identity));
    let identity = FrozenTypeIdentity::from_canonical_descriptor(&descriptor);
    ConstructorDescriptor {
        head_identity,
        descriptor,
        identity,
    }
}

/// Schema-name-checked storage getter for a reserved B4 carrier — the local
/// sibling of `frozen_identity_from_ref`, blocking forged carriers structurally
/// (the TraitRef/ImplRef precedent). `label` names the carrier in diagnostics.
fn reserved_storage<'a>(
    slot: &'a KindedSlot,
    expected_schema: &str,
    label: &str,
) -> Result<(TypeSchema, &'a TypedObjectStorage), String> {
    if slot.kind() != NativeKind::Ptr(HeapKind::TypedObject) {
        return Err(format!("expected a compiler-issued {label} value"));
    }
    let storage = slot
        .as_typed_object_storage()
        .ok_or_else(|| format!("received a null {label} value"))?;
    let schema = shape_runtime::type_schema::lookup_schema_by_id_public(storage.schema_id as u32)
        .ok_or_else(|| format!("could not resolve {label} schema id {}", storage.schema_id))?;
    if schema.name != expected_schema {
        return Err(format!(
            "expected {label}, got '{}' — only the compiler issues {label} values",
            schema.name
        ));
    }
    Ok((schema, storage))
}

/// Read one 128-bit identity from two int-half fields of a decoded carrier.
fn identity_halves_of(
    schema: &TypeSchema,
    storage: &TypedObjectStorage,
    high_field: &str,
    low_field: &str,
) -> Result<FrozenTypeIdentity, String> {
    let read = |name: &str| -> Result<i64, String> {
        let field = schema
            .get_field(name)
            .ok_or_else(|| format!("carrier schema '{}' has no {name} field", schema.name))?;
        storage
            .clone_field_kinded(field.index as usize)
            .and_then(|value| value.as_i64())
            .ok_or_else(|| format!("carrier field {name} is not an integer"))
    };
    Ok(FrozenTypeIdentity {
        high: read(high_field)?,
        low: read(low_field)?,
    })
}

/// ADR-009 B4: mint a `TypeConstructorRef` carrier for a frozen head identity.
/// The head is re-validated as a `Nominal` through the freeze (an unknown/
/// INVALID identity is the named unknown-constructor rejection R6, a non-nominal
/// the named non-Nominal rejection R5) so a forged or malformed head cannot
/// produce a usable constructor. Only the head identity halves are stored — the
/// ordered param kinds stay a freeze fact re-read at apply time (no second
/// table).
pub(crate) fn build_type_constructor_ref_heap_value(
    head_identity: FrozenTypeIdentity,
    freeze: &FreezeOverlay,
) -> Result<HeapValue, String> {
    if head_identity == FrozenTypeIdentity::INVALID {
        return Err("type_constructor received an unknown semantic type identity: the name is \
                    not a frozen nominal type constructor in this compilation unit"
            .to_string());
    }
    let category = freeze.category_of(head_identity)?;
    if category != FrozenTypeCategory::Nominal {
        return Err(format!(
            "type_constructor cannot build a constructor for this type: only nominal type \
             constructors accept type arguments (found category {})",
            category.variant_name()
        ));
    }
    typed_slot_into_heap_value(typed_object_for_named_schema(
        COMPTIME_FROZEN_TYPE_CONSTRUCTOR_REF_SCHEMA,
        &[
            ("identity_high", KindedSlot::from_int(head_identity.high)),
            ("identity_low", KindedSlot::from_int(head_identity.low)),
        ],
    ))
}

/// Read the frozen head identity out of a `TypeConstructorRef` carrier
/// (schema-name-checked).
fn type_constructor_head_from_ref(slot: &KindedSlot) -> Result<FrozenTypeIdentity, String> {
    let (schema, storage) = reserved_storage(
        slot,
        COMPTIME_FROZEN_TYPE_CONSTRUCTOR_REF_SCHEMA,
        "TypeConstructorRef",
    )?;
    identity_halves_of(&schema, storage, "identity_high", "identity_low")
}

/// ADR-009 B4: mint a checked const argument carrier (`const_arg(N)`). The
/// value canonicalizes to its own `const:int:{value}` identity (S1
/// [`canonical_const_arg`]); the carrier reuses the opaque `TypeRef` schema as
/// a pure identity transport (it never resolves as a frozen TYPE — `.apply`
/// classifies it as `Const` precisely because `category_of` rejects it, which
/// is what distinguishes it from a `type_ref` argument). Documented in
/// `docs/defections.md`.
pub(crate) fn build_const_arg_ref_heap_value(value: i64) -> Result<HeapValue, String> {
    let identity = canonical_const_arg(value).identity();
    typed_slot_into_heap_value(typed_object_for_named_schema(
        COMPTIME_FROZEN_TYPE_REF_SCHEMA,
        &[
            ("identity_high", KindedSlot::from_int(identity.high)),
            ("identity_low", KindedSlot::from_int(identity.low)),
        ],
    ))
}

/// Read one opaque argument's identity from a borrowed typed-object storage
/// (no owning `KindedSlot` is constructed, so nothing releases the borrowed
/// element on drop). Both `type_ref` and `const_arg` args carry the reserved
/// `TypeRef` schema — a foreign schema is a named rejection.
fn arg_identity_from_storage(storage: &TypedObjectStorage) -> Result<FrozenTypeIdentity, String> {
    let schema = shape_runtime::type_schema::lookup_schema_by_id_public(storage.schema_id as u32)
        .ok_or_else(|| {
            format!("could not resolve apply-argument schema id {}", storage.schema_id)
        })?;
    if schema.name != COMPTIME_FROZEN_TYPE_REF_SCHEMA {
        return Err(format!(
            "apply arguments must be checked type_ref / const_arg values, got '{}' — untyped \
             argument arrays cannot construct an application",
            schema.name
        ));
    }
    identity_halves_of(&schema, storage, "identity_high", "identity_low")
}

/// Classify a supplied argument identity by consulting the freeze: a frozen
/// TYPE identity is a `Type` argument, anything else (a `const_arg`'s
/// `const:int:{value}` identity) is a `Const` argument. `canonical_apply` then
/// checks the classified kind against the constructor's DECLARED param kind
/// (freeze authority) — this only decides which kind the argument *presents*,
/// never what the position *requires*.
fn classify_applied_arg(identity: FrozenTypeIdentity, freeze: &FreezeOverlay) -> AppliedArg {
    if freeze.category_of(identity).is_ok() {
        AppliedArg::Type(identity)
    } else {
        AppliedArg::Const(identity)
    }
}

/// Build the interleaved `high, low, …` int array of the ordered argument
/// identities for an `AppliedType` carrier.
fn build_arg_identity_array(args: &[FrozenTypeIdentity]) -> KindedSlot {
    let array = TypedArray::<i64>::with_capacity((args.len() * 2) as u32);
    // SAFETY: freshly allocated array pointer; stamping the element type +
    // pushing i64 halves mirrors the `frozen_erased_descriptor_slot` pattern.
    unsafe {
        stamp_elem_type(array as *mut u8, ELEM_TYPE_I64);
        for id in args {
            TypedArray::push(array, id.high);
            TypedArray::push(array, id.low);
        }
    }
    KindedSlot::new(
        ValueSlot::from_raw(array as usize as u64),
        NativeKind::Ptr(HeapKind::TypedArray),
    )
}

/// Read the ordered argument identities back out of an `AppliedType` carrier's
/// interleaved `high, low, …` int array.
fn read_arg_identity_array(slot: &KindedSlot) -> Result<Vec<FrozenTypeIdentity>, String> {
    if slot.kind() != NativeKind::Ptr(HeapKind::TypedArray) {
        return Err("AppliedType arg_identities field is not an array".to_string());
    }
    let ptr = slot.raw() as *const TypedArray<i64>;
    if ptr.is_null() {
        return Err("AppliedType arg_identities array is null".to_string());
    }
    // SAFETY: the kind witness + non-null check prove this is a live i64 array;
    // we only read halves, never take ownership.
    let out = unsafe {
        let len = TypedArray::len(ptr);
        if len % 2 != 0 {
            return Err("AppliedType arg_identities array has an odd length".to_string());
        }
        let mut out = Vec::with_capacity((len / 2) as usize);
        let mut i = 0;
        while i < len {
            let high = TypedArray::get(ptr, i).ok_or("AppliedType arg identity read out of range")?;
            let low =
                TypedArray::get(ptr, i + 1).ok_or("AppliedType arg identity read out of range")?;
            out.push(FrozenTypeIdentity { high, low });
            i += 2;
        }
        out
    };
    Ok(out)
}

/// Build an `AppliedType` carrier from a completed application.
fn build_applied_type_heap_value(
    applied_identity: FrozenTypeIdentity,
    head_identity: FrozenTypeIdentity,
    args: &[FrozenTypeIdentity],
) -> Result<HeapValue, String> {
    typed_slot_into_heap_value(typed_object_for_named_schema(
        COMPTIME_APPLIED_TYPE_SCHEMA,
        &[
            ("identity_high", KindedSlot::from_int(applied_identity.high)),
            ("identity_low", KindedSlot::from_int(applied_identity.low)),
            (
                "head_identity_high",
                KindedSlot::from_int(head_identity.high),
            ),
            ("head_identity_low", KindedSlot::from_int(head_identity.low)),
            ("arg_identities", build_arg_identity_array(args)),
        ],
    ))
}

/// The decoded triple carried by an `AppliedType` value.
struct DecodedApplication {
    head_identity: FrozenTypeIdentity,
    arg_identities: Vec<FrozenTypeIdentity>,
}

/// True when `slot` is an `AppliedType` carrier (schema-name-checked). Used by
/// `refine` to answer `None` for a non-applied receiver rather than erroring.
fn is_applied_type_carrier(slot: &KindedSlot) -> bool {
    if slot.kind() != NativeKind::Ptr(HeapKind::TypedObject) {
        return false;
    }
    let Some(storage) = slot.as_typed_object_storage() else {
        return false;
    };
    shape_runtime::type_schema::lookup_schema_by_id_public(storage.schema_id as u32)
        .map(|schema| schema.name == COMPTIME_APPLIED_TYPE_SCHEMA)
        .unwrap_or(false)
}

/// Schema-name-checked decode of an `AppliedType` carrier.
fn decode_applied_type(slot: &KindedSlot) -> Result<DecodedApplication, String> {
    let (schema, storage) = reserved_storage(slot, COMPTIME_APPLIED_TYPE_SCHEMA, "AppliedType")?;
    let head_identity =
        identity_halves_of(&schema, storage, "head_identity_high", "head_identity_low")?;
    let args_field = schema
        .get_field("arg_identities")
        .ok_or("AppliedType schema has no arg_identities field")?;
    let args_slot = storage
        .clone_field_kinded(args_field.index as usize)
        .ok_or("AppliedType arg_identities field is unreadable")?;
    let arg_identities = read_arg_identity_array(&args_slot)?;
    Ok(DecodedApplication {
        head_identity,
        arg_identities,
    })
}

/// ADR-009 B4: `constructor.apply(args)` — the uniform application entry. The
/// receiver head is decoded from the `TypeConstructorRef` carrier; each argument
/// is a checked `type_ref` / `const_arg` carrier read from the argument array
/// (an untyped array element is rejection R4). Arity + type-vs-const kind are
/// checked through the ONE freeze param-kind projection inside
/// [`canonical_apply`] (a generic enum head surfaces-and-stops there); the
/// resulting identity EQUALS the A2 `type_ref(Head<Args>)` spelling.
pub(crate) fn apply_to_constructor(
    receiver: &KindedSlot,
    args_array: &KindedSlot,
    freeze: &FreezeOverlay,
) -> Result<HeapValue, String> {
    let head_identity = type_constructor_head_from_ref(receiver)?;
    let constructor = constructor_descriptor_from_head(head_identity);

    // Decode the argument array element-by-element from borrowed storages.
    if args_array.kind() != NativeKind::Ptr(HeapKind::TypedArray) {
        return Err("apply expects its arguments as a checked array of type_ref / const_arg \
                    values"
            .to_string());
    }
    let array_ptr = args_array.raw() as *const TypedArray<*const TypedObjectStorage>;
    if array_ptr.is_null() {
        return Err("apply received a null argument array".to_string());
    }
    // SAFETY: kind witness + non-null check prove this is a live typed-object
    // array; each element is a borrowed storage pointer (no ownership taken).
    let mut applied_args = Vec::new();
    let mut arg_identities = Vec::new();
    unsafe {
        let len = TypedArray::len(array_ptr);
        for i in 0..len {
            let elem = TypedArray::get(array_ptr, i)
                .ok_or("apply argument array read out of range")?;
            if elem.is_null() {
                return Err("apply received a null argument".to_string());
            }
            let identity = arg_identity_from_storage(&*elem)?;
            applied_args.push(classify_applied_arg(identity, freeze));
            arg_identities.push(identity);
        }
    }

    let applied = canonical_apply(&constructor, &applied_args, freeze)?;
    build_applied_type_heap_value(applied.identity, head_identity, &arg_identities)
}

/// ADR-009 B4: `applied.refine(constructor)` — round-trips ONLY over a genuine
/// application whose head matches the constructor. Returns `Some(AppliedType)`
/// on a head match, `None` otherwise (never an error, never partial — R7).
pub(crate) fn refine_application(
    applied: &KindedSlot,
    constructor: &KindedSlot,
) -> Result<Option<HeapValue>, String> {
    // R7: refine round-trips ONLY over a genuine application. A bare-nominal
    // `type_ref(...)` (or any non-AppliedType carrier) has no stored structure
    // to recover — that is `None`, never an error and never a partial answer.
    if !is_applied_type_carrier(applied) {
        return Ok(None);
    }
    let decoded = decode_applied_type(applied)?;
    let head_identity = type_constructor_head_from_ref(constructor)?;
    if decoded.head_identity != head_identity {
        return Ok(None);
    }
    // Recover the canonical applied identity so the returned carrier is
    // identity-EQUAL to the input (round-trip stability). Re-mint from the
    // stored head + args rather than re-reading the input's identity field, so
    // the descriptor grammar stays the single source.
    let embedded: Vec<String> = decoded
        .arg_identities
        .iter()
        .map(|id| identity_hex(*id))
        .collect();
    let applied_identity = FrozenTypeIdentity::from_canonical_descriptor(&format!(
        "applied:{}<{}>",
        identity_hex(head_identity),
        embedded.join(",")
    ));
    Ok(Some(build_applied_type_heap_value(
        applied_identity,
        head_identity,
        &decoded.arg_identities,
    )?))
}

/// ADR-009 B4: `applied.type_argument(index)` — the frozen identity of the
/// `index`-th argument, re-issued as a `TypeRef`. A checked const index; an
/// out-of-range index is the named rejection (S1 [`type_argument`]). A const
/// argument at that position rejects at the `TypeRef` builder (it is not a
/// frozen value type) — `type_argument` recovers TYPE arguments.
pub(crate) fn applied_type_argument(
    applied: &KindedSlot,
    index: i64,
    freeze: &FreezeOverlay,
) -> Result<HeapValue, String> {
    let decoded = decode_applied_type(applied)?;
    let refined = RefinedApplication {
        head_identity: decoded.head_identity,
        arg_identities: decoded.arg_identities,
    };
    if index < 0 {
        return Err(format!(
            "type_argument index {index} is out of range: index must be non-negative"
        ));
    }
    let identity = type_argument(&refined, index as usize)?;
    build_frozen_type_ref_heap_value(identity, freeze)
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

/// Read the frozen semantic identity out of an opaque `TypeRef` argument
/// slot. The ONE TypeRef-argument reader shared by every TypeRef-consuming
/// intrinsic (`type_category`, `reflect` — ADR-009 B1 S3); `caller` names
/// the intrinsic in each R4 diagnostic ("<caller> expects a TypeRef value"
/// family), so both intrinsics reject malformed arguments identically.
fn frozen_identity_from_ref(slot: &KindedSlot, caller: &str) -> Result<FrozenTypeIdentity, String> {
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
