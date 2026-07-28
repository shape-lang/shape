use super::semantic_freeze::{FreezeOverlay, annotation_has_unresolved_inference_variable};
use sha2::{Digest, Sha256};
use shape_ast::ast::{ObjectTypeField, TypeAnnotation};
pub(crate) use shape_runtime::comptime_reflection::FrozenTypeCategory;
use shape_runtime::comptime_reflection::{
    FloatWidth, FrozenPrimitive, IntegerWidth, ParamKind, PassingMode,
};
use shape_runtime::type_schema::TypeSchema;
use shape_runtime::type_schema::builtin_schemas::{
    COMPTIME_APPLIED_TYPE_SCHEMA, COMPTIME_FROZEN_TYPE_CONSTRUCTOR_REF_SCHEMA,
    COMPTIME_FROZEN_TYPE_REF_SCHEMA, COMPTIME_REPRESENTATION_ACCESS_SCHEMA,
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

/// ADR-009 E1 #17 (slice 5, A-FULL): invert the ONE [`PRIMITIVE_SYNONYM_FAMILIES`]
/// table — the canonical (`names[0]`) spelling for a frozen primitive. The type-ref
/// reconstruction path (`reconstruct_type_annotation`) reads THIS, never a second
/// name table (E1-D7(c)): the same table that classifies leaf names forward also
/// spells them back, so a synonym (`str`, `i64`, `f64`) always reconstructs to its
/// family's canonical form (`string`, `int`, `number`). `None` iff a new
/// `FrozenPrimitive` variant is added without a table row — a named rejection at
/// the caller, never a guessed spelling.
pub(crate) fn canonical_primitive_spelling(primitive: FrozenPrimitive) -> Option<&'static str> {
    PRIMITIVE_SYNONYM_FAMILIES
        .iter()
        .find(|(_, family_primitive)| *family_primitive == primitive)
        .and_then(|(names, _)| names.first().copied())
}

/// ADR-009 B6 R2 (Dec 63): a callable's parameters are heterogeneous
/// signature-indexed descriptors, not a homogeneous top-typed collection.
/// Modeling a signature parameter with the compiler-internal `Any` top type
/// (bare `Any` or `Array<Any>`) erases the per-position type — the named
/// rejection, in the same Any-erasure family as the B3
/// `WITNESS_ERASED_TO_ANY_DIAGNOSTIC` (lowercase `any` stays the enabled Erased
/// leaf; only capital `Any` is refused).
pub(crate) const CALLABLE_PARAM_ERASED_TO_ANY_DIAGNOSTIC: &str = "a callable's parameters are heterogeneous signature-indexed descriptors, not a \
     homogeneous Any collection: a parameter cannot be typed with the compiler-internal \
     Any top type (use a concrete per-position type; lowercase `any` is the enabled \
     Erased leaf)";

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
    /// Named freeze input 3 (enriched, ADR-009 B5): ordered enum variants per
    /// user enum name — the variant NAME (source of the owner-bound hygienic
    /// member identity, Dec 57) plus the variant's payload ARITY (Unit=0,
    /// Tuple(n)/Struct(n)=n). The single enum freeze projection; full per-variant
    /// payload field TYPES are a later B5 slice (documented in defections.md).
    pub(crate) enum_defs: HashMap<String, Vec<FrozenEnumVariantDef>>,
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
    /// ADR-009 B5 (S2, Dec 55 R10): the declared ordered generic parameter
    /// NAMES per user STRUCT name — the NAME projection of the SAME freeze
    /// input 1 (`struct_generic_info.type_params`) whose KIND projection is
    /// [`Self::struct_generic_param_kinds`] (two projections of one fact, not a
    /// second fact). Read ONLY by [`Self::substituted_applied_nominal`] to bind
    /// each declared parameter to its applied argument identity before
    /// re-canonicalizing the struct's field annotations.
    pub(crate) struct_generic_param_names: HashMap<String, Vec<String>>,
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
    /// ADR-009 B6 (Dec 63): the ordered structural descriptor per base-interned
    /// `Callable` identity (a module-level alias whose target is a callable
    /// type, e.g. `type Handler = (int) -> bool`). Populated by the alias
    /// fixpoint in the SAME rebuild that interns the identity (one source, no
    /// second derivation), so `payload_for_identity` answers the complete
    /// `FrozenCallable` — never a partial descriptor. Symmetric with
    /// [`Self::frozen_primitive_payloads`].
    pub(crate) frozen_callable_descriptors:
        HashMap<FrozenTypeIdentity, payloads::CallableDescriptor>,
    /// ADR-009 B5 (Dec 55): the sealed nominal declaration-shape descriptor per
    /// RESOLVED nominal identity (user struct / enum), reconstructed from the
    /// enriched struct/enum freeze inputs 1+3 in the SAME rebuild that interns
    /// the identity (one source, no second derivation). `payload_for_identity`
    /// answers the complete `FrozenNominal` from here; an un-applied generic
    /// head or an unsubstituted applied form has NO entry and is a named
    /// rejection. Symmetric with [`Self::frozen_callable_descriptors`].
    pub(crate) frozen_nominal_descriptors: HashMap<FrozenTypeIdentity, payloads::NominalDescriptor>,
    /// ADR-009 E5 CKPT-2 (A8-OUT): the static builtin-nominal declaration-shape
    /// template per builtin generic HEAD identity (`Array`, `Option`, `Result`,
    /// …). NOT a per-type freeze fact — it is STATIC builtin data, populated in
    /// the SAME 11-head intern loop that already interns the builtin arity (one
    /// source, no second derivation), keyed by the interned head identity.
    /// [`Self::substituted_applied_nominal`] reads it to answer a COMPLETE
    /// descriptor for an APPLIED builtin (`Array<int>` ⇒ `Opaque`,
    /// `Result<T,E>` ⇒ `Enum`) without inverting the identity hash. Under
    /// A8-OUT the template carries NO payload TYPES: a container states no
    /// fields (Opaque — none to mis-state), an enum states TRUE variant names +
    /// arities; every applied type ARGUMENT is recovered by the orthogonal
    /// [`type_argument`] query (A7-uniform), never fabricated into the
    /// descriptor.
    pub(crate) builtin_nominal_templates: HashMap<FrozenTypeIdentity, BuiltinNominalTemplate>,
    /// ADR-009 E5 CKPT-2 (F2): the refined APPLIED form recovered for a
    /// BASE-interned applied-nominal identity — an alias whose target is an
    /// applied generic (`type Ints = Array<int>`, `type PageOfInt = Page<int>`).
    /// Populated by the alias fixpoint in the SAME rebuild that interns the
    /// alias's transparent applied identity (write-once, symmetric with the
    /// composite-descriptor threading). Read ONLY by the base `Nominal` arm of
    /// [`Self::payload_for_identity`] (before the pending rejection) to
    /// substitute lazily via [`Self::substituted_applied_nominal`] — symmetric
    /// with the overlay memo's `applied_nominal` arm, so alias-of-applied
    /// reflects exactly as the direct applied form does (no reflect asymmetry).
    pub(crate) base_applied_nominals: HashMap<FrozenTypeIdentity, RefinedApplication>,
    /// ADR-009 B7 (Dec 50/94): the composite structural descriptors per
    /// BASE-interned composite identity — an alias whose target is a composite
    /// (`type Pair = [int, string]`, `type Ref = &User`, `type Id = int |
    /// string`, `type Row = {x: int}`). Populated by the alias fixpoint in the
    /// SAME rebuild that interns the identity (one source, no second
    /// derivation), so `payload_for_identity` answers the complete composite
    /// payload — never a partial descriptor. Symmetric with
    /// [`Self::frozen_callable_descriptors`].
    pub(crate) frozen_tuple_descriptors: HashMap<FrozenTypeIdentity, payloads::TupleDescriptor>,
    /// See [`Self::frozen_tuple_descriptors`].
    pub(crate) frozen_record_descriptors: HashMap<FrozenTypeIdentity, payloads::RecordDescriptor>,
    /// See [`Self::frozen_tuple_descriptors`].
    pub(crate) frozen_reference_descriptors:
        HashMap<FrozenTypeIdentity, payloads::ReferenceDescriptor>,
    /// See [`Self::frozen_tuple_descriptors`].
    pub(crate) frozen_union_descriptors: HashMap<FrozenTypeIdentity, payloads::UnionDescriptor>,
}

/// ADR-009 B5 (Dec 57) — one enum variant freeze projection: the source-level
/// variant NAME (hashed into the owner-bound hygienic member identity, never
/// exposed as a selectable string) and the variant's payload ARITY.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FrozenEnumVariantDef {
    pub(crate) name: String,
    pub(crate) payload_arity: u16,
}

/// ADR-009 E5 CKPT-2 (A8-OUT) — the static reflection template for one builtin
/// generic HEAD. `canonical_name` mints the owner-bound hygienic member
/// identity (`member:{owner}:{variant}`) for each enum variant — the SAME
/// identity a monomorphized declaration of the builtin enum would mint. NOT a
/// per-type freeze fact: static builtin data keyed by the interned head
/// identity (see [`FrozenTypeIndex::builtin_nominal_templates`]).
#[derive(Debug, Clone, Copy)]
pub(crate) struct BuiltinNominalTemplate {
    pub(crate) canonical_name: &'static str,
    pub(crate) shape: BuiltinShape,
}

/// ADR-009 E5 CKPT-2 (A8-OUT) — the two builtin reflection shapes.
#[derive(Debug, Clone, Copy)]
pub(crate) enum BuiltinShape {
    /// A homogeneous container (`Array`/`Vec`/`Set`/`HashMap`/`Deque`/
    /// `PriorityQueue`/`Mutex`/`Slice`/`Future`): reflects as a
    /// non-decomposable `Opaque` — it states NO named-field/variant structure
    /// (there is none to state, so nothing to mis-state); its element/key/value
    /// types are recovered via [`type_argument`], never fabricated into the
    /// descriptor.
    Container,
    /// A builtin sum type (`Option`/`Result`): reflects as an `Enum` stating its
    /// TRUE variant names + arities. Under A8-OUT payload TYPES are NOT stated
    /// here (a swap like `Result<int,string>` vs `Result<string,int>` produces
    /// IDENTICAL descriptors); the payloads are recovered via [`type_argument`].
    Enum(&'static [BuiltinVariant]),
}

/// ADR-009 E5 CKPT-2 (A8-OUT) — one builtin enum variant: its source variant
/// NAME (hashed into the owner-bound hygienic member identity, never exposed as
/// a selectable string) and its payload ARITY (Unit=0, Tuple(n)/Struct(n)=n).
#[derive(Debug, Clone, Copy)]
pub(crate) struct BuiltinVariant {
    pub(crate) name: &'static str,
    pub(crate) arity: u16,
}

/// ADR-009 E5 CKPT-2 (A8-OUT) — `Option` reflects as a two-variant `Enum`
/// (`None` unit / `Some(_)`), the payload recovered via [`type_argument`].
const OPTION_BUILTIN_VARIANTS: &[BuiltinVariant] = &[
    BuiltinVariant {
        name: "None",
        arity: 0,
    },
    BuiltinVariant {
        name: "Some",
        arity: 1,
    },
];

/// ADR-009 E5 CKPT-2 (A8-OUT) — `Result` reflects as a two-variant `Enum`
/// (`Ok(_)` / `Err(_)`), both payloads recovered via [`type_argument`] (arg 0 =
/// `Ok`'s payload, arg 1 = `Err`'s).
const RESULT_BUILTIN_VARIANTS: &[BuiltinVariant] = &[
    BuiltinVariant {
        name: "Ok",
        arity: 1,
    },
    BuiltinVariant {
        name: "Err",
        arity: 1,
    },
];

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
            // ADR-009 B6: a base-interned callable alias answers its complete
            // signature descriptor from the structural map populated in the
            // same rebuild — never a partial descriptor. Interning without the
            // structure is an internal-invariant violation.
            FrozenTypeCategory::Callable => self
                .frozen_callable_descriptors
                .get(&identity)
                .cloned()
                .map(FrozenPayloadDescriptor::Callable)
                .ok_or_else(|| {
                    "internal invariant: a Callable identity was frozen without its \
                     FrozenCallable structural descriptor"
                        .to_string()
                }),
            // ADR-009 B5: a RESOLVED nominal (user struct/enum) answers its
            // complete declaration-shape descriptor from the map populated in
            // the same rebuild. A frozen NOMINAL identity WITHOUT a descriptor
            // is either an un-applied generic constructor head (it has declared
            // param kinds — B4 `TypeConstructorRef` territory) or an
            // unsubstituted APPLIED form (generic substitution pending) — each a
            // distinct named rejection, never a partial descriptor.
            FrozenTypeCategory::Nominal => {
                if let Some(descriptor) = self.frozen_nominal_descriptors.get(&identity) {
                    Ok(FrozenPayloadDescriptor::Nominal(descriptor.clone()))
                } else if self.generic_param_kinds.contains_key(&identity) {
                    // A declared generic HEAD (builtin `Array` / a generic user
                    // struct — incl. the F3 Phantom case now excluded from
                    // `frozen_nominal_descriptors`): un-applied → the named
                    // A3 rejection.
                    Err(payloads::unapplied_generic_head_rejection())
                } else if let Some(descriptor) =
                    self.base_applied_nominals
                        .get(&identity)
                        .and_then(|applied| {
                            self.substituted_applied_nominal(
                                applied.head_identity,
                                &applied.arg_identities,
                            )
                        })
                {
                    // ADR-009 E5 CKPT-2 (F2): a BASE-interned alias-of-applied
                    // (`type Ints = Array<int>`, `type PageOfInt = Page<int>`)
                    // resolves to the transparent applied identity; substitute
                    // lazily via the SAME method the overlay memo arm uses, so
                    // alias-of-applied reflects exactly as the direct applied
                    // form (no reflect asymmetry).
                    Ok(FrozenPayloadDescriptor::Nominal(descriptor))
                } else {
                    Err(payloads::applied_nominal_pending_rejection())
                }
            }
            // ADR-009 B7: a base-interned composite alias answers its complete
            // structural descriptor from the per-category map populated in the
            // same rebuild — never a partial descriptor. Interning a composite
            // identity without its descriptor is an internal-invariant violation.
            FrozenTypeCategory::Tuple => self
                .frozen_tuple_descriptors
                .get(&identity)
                .cloned()
                .map(FrozenPayloadDescriptor::Tuple)
                .ok_or_else(|| composite_without_descriptor_invariant("Tuple")),
            FrozenTypeCategory::Record => self
                .frozen_record_descriptors
                .get(&identity)
                .cloned()
                .map(FrozenPayloadDescriptor::Record)
                .ok_or_else(|| composite_without_descriptor_invariant("Record")),
            FrozenTypeCategory::Reference => self
                .frozen_reference_descriptors
                .get(&identity)
                .copied()
                .map(FrozenPayloadDescriptor::Reference)
                .ok_or_else(|| composite_without_descriptor_invariant("Reference")),
            FrozenTypeCategory::Union => self
                .frozen_union_descriptors
                .get(&identity)
                .cloned()
                .map(FrozenPayloadDescriptor::Union)
                .ok_or_else(|| composite_without_descriptor_invariant("Union")),
            // Only `Parameter` (a scoped overlay identity — never base-interned)
            // and `Existential` (its witness payload lands with B3-S3) remain
            // the named per-category rejections at the base index.
            pending => Err(payloads::pending_payload_rejection(pending)),
        }
    }

    /// ADR-009 B5 (S2, Dec 55 R10): generic substitution PRECEDES descriptor
    /// issuance. For an APPLIED user-struct form (`Page<User>`, decomposed by
    /// [`canonical_refine`] into the head identity + ordered argument
    /// identities), bind each declared struct type parameter to its applied
    /// argument identity and re-canonicalize the struct's field annotations
    /// through the ONE canonicalizer — so a field spelled `Array<T>` freezes as
    /// `Array<User>`, never the un-substituted `T`. The classification is the
    /// SAME field-count rule the base rebuild uses (0 → Opaque, 1 → Newtype,
    /// ≥2 → Struct), so an applied form and a hypothetical monomorphized
    /// declaration would agree.
    ///
    /// ADR-009 E5 CKPT-2 (A8-OUT): BEFORE the user-struct path, two additive
    /// branches answer the two other applicable-head families with a COMPLETE,
    /// NON-FABRICATING descriptor (removing the reflect asymmetry where an
    /// applied builtin/enum pended while an applied struct substituted):
    ///
    /// * **Branch A — a builtin generic head** (`Array<int>`, `Result<T,E>`, …):
    ///   the static [`Self::builtin_nominal_templates`] answer. A container ⇒
    ///   `Opaque` (no fields — none to mis-state); `Option`/`Result` ⇒ `Enum`
    ///   with TRUE variant names + arities. Under A8-OUT NO payload TYPE is
    ///   stated — every applied argument (container element/key/value AND enum
    ///   payload) is recovered by the orthogonal [`type_argument`] query
    ///   (A7-uniform), never fabricated into the descriptor.
    /// * **Branch B — a user ENUM head** (`Maybe<int>`, `Either<L,R>`): reuse the
    ///   arity-only base enum descriptor. SOUND under A8-OUT because that
    ///   descriptor is param-AGNOSTIC (member identities + arities are
    ///   name-derived, `T`-free) — the base head descriptor already IS the
    ///   applied answer; enum payload TYPES are recovered via [`type_argument`].
    ///
    /// `owner: head` in every branch (never the applied identity): `Array<int>`
    /// and `Array<string>` share owner and are distinguished only via the
    /// applied identity + [`type_argument`].
    ///
    /// Returns `None` (→ the caller's named applied-substitution-pending
    /// rejection) when the head is neither a builtin template, a user enum, nor
    /// a resolved user struct with frozen parameter names, the arity does not
    /// match (struct path), or any substituted field type fails to canonicalize
    /// — never a partial descriptor (R8).
    pub(super) fn substituted_applied_nominal(
        &self,
        head: FrozenTypeIdentity,
        args: &[FrozenTypeIdentity],
    ) -> Option<payloads::NominalDescriptor> {
        // Branch A — a builtin generic head (container or Option/Result): the
        // static builtin template. Arity-agnostic and arg-agnostic: a container
        // states no fields, an enum states name-derived variants — no applied
        // argument is ever read into the descriptor (A7: every arg lives in the
        // orthogonal `type_argument` query), so the answer is SOUND for any args.
        if let Some(template) = self.builtin_nominal_templates.get(&head) {
            return Some(match template.shape {
                BuiltinShape::Container => payloads::NominalDescriptor::Opaque { owner: head },
                BuiltinShape::Enum(variants) => payloads::NominalDescriptor::Enum {
                    owner: head,
                    variants: variants
                        .iter()
                        .map(|variant| payloads::NominalVariantDescriptor {
                            member: member_identity(template.canonical_name, variant.name),
                            payload_arity: variant.arity,
                        })
                        .collect(),
                },
            });
        }
        // Branch B — a user ENUM head: reuse the arity-only base enum descriptor.
        // Param-agnostic (member ids + arities are `T`-free), so the base
        // `Maybe`-head descriptor IS the applied `Maybe<int>` answer; the enum
        // payload TYPES are recovered via `type_argument`. A user STRUCT head
        // (`Struct`/`Newtype`/`Opaque` in the map, or excluded generic head)
        // falls through to the real 5-step substitution below.
        if let Some(descriptor @ payloads::NominalDescriptor::Enum { .. }) =
            self.frozen_nominal_descriptors.get(&head)
        {
            return Some(descriptor.clone());
        }

        // Reverse the head identity to the user-struct name (the freeze keys
        // struct definitions by name; only a user struct carries field
        // annotations to substitute — a builtin/enum head has no `struct_defs`
        // entry and returns `None` here, staying the pending rejection).
        let name = self
            .struct_defs
            .keys()
            .find(|name| self.frozen_type_ids.get(*name) == Some(&head))?
            .clone();
        let param_names = self.struct_generic_param_names.get(&name)?;
        if param_names.len() != args.len() || args.is_empty() {
            return None;
        }
        let fields = self.struct_defs.get(&name)?;

        // The substitution scope: a declared parameter resolves to its applied
        // argument identity (with the argument's frozen category); every other
        // leaf resolves through the base freeze exactly as the un-applied
        // rebuild does. One canonicalizer, one leaf-resolution surface.
        let substitution: HashMap<&str, FrozenTypeIdentity> = param_names
            .iter()
            .map(String::as_str)
            .zip(args.iter().copied())
            .collect();
        let resolve = |leaf: &str| {
            if let Some(&arg) = substitution.get(leaf) {
                let category = self.frozen_type_categories.get(&arg).copied()?;
                return Some((arg, category));
            }
            let identity = self.frozen_type_ids.get(leaf).copied()?;
            let category = self.frozen_type_categories.get(&identity).copied()?;
            Some((identity, category))
        };
        let is_trait = |leaf: &str| self.trait_names.contains(leaf);
        let applied_arity =
            |identity: FrozenTypeIdentity| self.generic_param_kinds.get(&identity).map(Vec::len);
        let scope = LeafScope {
            resolve: &resolve,
            is_trait: &is_trait,
            applied_arity: &applied_arity,
        };

        let mut field_descriptors = Vec::with_capacity(fields.len());
        for (field_name, annotation) in fields {
            let canonical = canonicalize_with(annotation, &scope).ok()?;
            field_descriptors.push(payloads::NominalFieldDescriptor {
                member: member_identity(&name, field_name),
                type_identity: canonical.identity,
                initialization: shape_runtime::comptime_reflection::FieldInitialization::Required,
            });
        }

        Some(match field_descriptors.as_slice() {
            [] => payloads::NominalDescriptor::Opaque { owner: head },
            [single] => payloads::NominalDescriptor::Newtype {
                owner: head,
                inner: single.type_identity,
            },
            _ => payloads::NominalDescriptor::Struct {
                owner: head,
                fields: field_descriptors,
            },
        })
    }

    pub(super) fn rebuild_frozen_type_index(&mut self) {
        let mut ids = HashMap::new();
        let mut categories = HashMap::new();
        let mut primitive_payloads = HashMap::new();
        let mut param_kinds: HashMap<FrozenTypeIdentity, Vec<ParamKind>> = HashMap::new();
        // ADR-009 E5 CKPT-2 (A8-OUT): the builtin declaration-shape templates,
        // populated in the SAME 11-head intern loop that interns builtin arity.
        let mut builtin_templates: HashMap<FrozenTypeIdentity, BuiltinNominalTemplate> =
            HashMap::new();
        // ADR-009 E5 CKPT-2 (F2): the refined applied form per BASE-interned
        // alias-of-applied identity, threaded in the alias fixpoint below.
        let mut base_applied_nominals: HashMap<FrozenTypeIdentity, RefinedApplication> =
            HashMap::new();
        let mut callable_descriptors: HashMap<FrozenTypeIdentity, payloads::CallableDescriptor> =
            HashMap::new();
        // ADR-009 B7: per-category descriptor maps for BASE-interned composite
        // aliases, threaded in the alias fixpoint below (write-once, same round
        // as the identity intern).
        let mut tuple_descriptors: HashMap<FrozenTypeIdentity, payloads::TupleDescriptor> =
            HashMap::new();
        let mut record_descriptors: HashMap<FrozenTypeIdentity, payloads::RecordDescriptor> =
            HashMap::new();
        let mut reference_descriptors: HashMap<FrozenTypeIdentity, payloads::ReferenceDescriptor> =
            HashMap::new();
        let mut union_descriptors: HashMap<FrozenTypeIdentity, payloads::UnionDescriptor> =
            HashMap::new();

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

        // Builtin nominal constructors: one table carries name, declared arity
        // (S5 R5 — arity is a freeze fact, enforced by the single canonicalizer,
        // identity-keyed so alias heads inherit it) AND its A8-OUT reflection
        // SHAPE (E5 CKPT-2). Every builtin generic parameter is a TYPE parameter
        // (ADR-009 B4, Dec 54): no builtin declares a const generic, so arity
        // `n` projects to `[ParamKind::Type; n]` — arity is the vector length.
        // The `BuiltinShape` column feeds `builtin_nominal_templates` in the
        // SAME insert (one source, no second table): 9 containers → `Opaque`,
        // `Option`/`Result` → arity-only `Enum` (payloads via `type_argument`).
        for (name, arity, shape) in [
            ("Array", 1, BuiltinShape::Container),
            ("Vec", 1, BuiltinShape::Container),
            ("HashMap", 2, BuiltinShape::Container),
            ("Option", 1, BuiltinShape::Enum(OPTION_BUILTIN_VARIANTS)),
            ("Result", 2, BuiltinShape::Enum(RESULT_BUILTIN_VARIANTS)),
            ("Future", 1, BuiltinShape::Container),
            ("Set", 1, BuiltinShape::Container),
            ("Deque", 1, BuiltinShape::Container),
            ("PriorityQueue", 1, BuiltinShape::Container),
            ("Mutex", 1, BuiltinShape::Container),
            ("Slice", 1, BuiltinShape::Container),
        ] {
            let identity = intern_identity(
                &mut ids,
                &mut categories,
                name,
                &format!("nominal:{name}"),
                FrozenTypeCategory::Nominal,
            );
            param_kinds.insert(identity, vec![ParamKind::Type; arity]);
            builtin_templates.insert(
                identity,
                BuiltinNominalTemplate {
                    canonical_name: name,
                    shape,
                },
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
                // ADR-009 B6: preserve a callable alias's structural descriptor
                // in the same rebuild that interns its identity (write-once —
                // composite identities never re-hash across rounds).
                if let Some(descriptor) = canonical.callable.clone() {
                    callable_descriptors
                        .entry(canonical.identity)
                        .or_insert(descriptor);
                }
                // ADR-009 B7: same write-once discipline for the four composite
                // structural descriptors, so a base-interned composite alias
                // answers its complete payload from `payload_for_identity`.
                if let Some(descriptor) = canonical.tuple.clone() {
                    tuple_descriptors
                        .entry(canonical.identity)
                        .or_insert(descriptor);
                }
                if let Some(descriptor) = canonical.record.clone() {
                    record_descriptors
                        .entry(canonical.identity)
                        .or_insert(descriptor);
                }
                if let Some(descriptor) = canonical.reference {
                    reference_descriptors
                        .entry(canonical.identity)
                        .or_insert(descriptor);
                }
                if let Some(descriptor) = canonical.union.clone() {
                    union_descriptors
                        .entry(canonical.identity)
                        .or_insert(descriptor);
                }
                // ADR-009 E5 CKPT-2 (F2): an alias whose target is an APPLIED
                // generic (`type Ints = Array<int>`, `type PageOfInt =
                // Page<int>`) receives the target's transparent applied identity.
                // Preserve the refined application (head + ordered args) keyed by
                // that identity — the SAME write-once discipline as the composite
                // descriptors — so the base `Nominal` arm can substitute lazily
                // and alias-of-applied reflects exactly as the direct applied
                // form. `canonical_refine` returns `Some` ONLY for a genuine
                // `applied:` descriptor, so bare/composite aliases never store.
                if let Some(refined) = canonical_refine(&canonical.descriptor) {
                    base_applied_nominals
                        .entry(canonical.identity)
                        .or_insert(refined);
                }
                changed |=
                    ids.insert((*alias).clone(), canonical.identity) != Some(canonical.identity);
            }
            if !changed {
                break;
            }
        }

        // ADR-009 B5 (Dec 55/57): reconstruct the sealed nominal declaration
        // shape per RESOLVED user nominal from the enriched struct/enum freeze
        // inputs 1+3 — the SAME rebuild that interned the identity (one source,
        // no second derivation). Runs AFTER the alias fixpoint so struct field
        // types resolve against the fully-interned base (leaf/applied/composite
        // members). A struct whose field type cannot canonicalize is SKIPPED (no
        // partial descriptor, R8) — reflecting it stays an applied/pending
        // rejection rather than issuing a half-populated shape.
        let mut nominal_descriptors: HashMap<FrozenTypeIdentity, payloads::NominalDescriptor> =
            HashMap::new();
        {
            let resolve = |name: &str| {
                let identity = ids.get(name).copied()?;
                let category = categories.get(&identity).copied()?;
                Some((identity, category))
            };
            let is_trait = |name: &str| self.trait_names.contains(name);
            let applied_arity =
                |identity: FrozenTypeIdentity| param_kinds.get(&identity).map(Vec::len);
            let scope = LeafScope {
                resolve: &resolve,
                is_trait: &is_trait,
                applied_arity: &applied_arity,
            };

            for (name, fields) in &self.struct_defs {
                let Some(owner) = ids.get(name).copied() else {
                    continue;
                };
                // ADR-009 E5 CKPT-2 (F3 — the Phantom guard): a struct that
                // declares NON-EMPTY generic parameters (a generic HEAD) gets NO
                // base monomorphic descriptor. Without this, a generic head whose
                // fields do not reference the parameter (`type Phantom<T>{tag:int}`
                // — all fields canonicalize under the base freeze) would land a
                // `Struct{tag:int}` descriptor and both reflect (`payload_of`) and
                // spell (`bare_nominal_name_of`) as if MONOMORPHIC, bypassing the
                // A3 un-applied-generic-head rejection. An applied form
                // (`Phantom<int>`) still substitutes via
                // `substituted_applied_nominal` (which reads `struct_defs` +
                // `struct_generic_param_names`, not this map); the bare head stays
                // the named `unapplied_generic_head_rejection` (it is in
                // `generic_param_kinds` with a non-empty vec). `Box<T>{value:T}`
                // was already excluded (its `value:T` field fails base
                // canonicalization). The `is_empty` check is load-bearing: EVERY
                // struct in `struct_generic_info` gets a `struct_generic_param_kinds`
                // entry (empty for a non-generic struct), so `contains_key` alone
                // would wrongly exclude monomorphic structs.
                if self
                    .struct_generic_param_kinds
                    .get(name)
                    .is_some_and(|kinds| !kinds.is_empty())
                {
                    continue;
                }
                let mut field_descriptors = Vec::with_capacity(fields.len());
                let mut all_resolved = true;
                for (field_name, annotation) in fields {
                    let Ok(canonical) = canonicalize_with(annotation, &scope) else {
                        all_resolved = false;
                        break;
                    };
                    field_descriptors.push(payloads::NominalFieldDescriptor {
                        member: member_identity(name, field_name),
                        type_identity: canonical.identity,
                        // ADR-009 B5 S1: the struct freeze input does not carry
                        // per-field default flags, so every field is `Required`
                        // today (Dec 59 total records — a default is
                        // construction policy only). `Defaulted` population is a
                        // later slice (documented in defections.md).
                        initialization:
                            shape_runtime::comptime_reflection::FieldInitialization::Required,
                    });
                }
                if !all_resolved {
                    continue;
                }
                // Field-count classification (S1 CURRENT, absent dedicated
                // newtype/opaque syntax — documented in defections.md): 0 fields
                // is a non-decomposable Opaque, exactly 1 field is a Newtype
                // wrapper over its inner type, ≥2 fields is a Struct.
                let descriptor = match field_descriptors.as_slice() {
                    [] => payloads::NominalDescriptor::Opaque { owner },
                    [single] => payloads::NominalDescriptor::Newtype {
                        owner,
                        inner: single.type_identity,
                    },
                    _ => payloads::NominalDescriptor::Struct {
                        owner,
                        fields: field_descriptors,
                    },
                };
                nominal_descriptors.insert(owner, descriptor);
            }

            for (name, variants) in &self.enum_defs {
                let Some(owner) = ids.get(name).copied() else {
                    continue;
                };
                let variant_descriptors = variants
                    .iter()
                    .map(|variant| payloads::NominalVariantDescriptor {
                        member: member_identity(name, &variant.name),
                        payload_arity: variant.payload_arity,
                    })
                    .collect();
                nominal_descriptors.insert(
                    owner,
                    payloads::NominalDescriptor::Enum {
                        owner,
                        variants: variant_descriptors,
                    },
                );
            }
        }

        self.frozen_type_ids = ids;
        self.frozen_type_categories = categories;
        self.frozen_primitive_payloads = primitive_payloads;
        self.generic_param_kinds = param_kinds;
        self.builtin_nominal_templates = builtin_templates;
        self.base_applied_nominals = base_applied_nominals;
        self.frozen_callable_descriptors = callable_descriptors;
        self.frozen_nominal_descriptors = nominal_descriptors;
        self.frozen_tuple_descriptors = tuple_descriptors;
        self.frozen_record_descriptors = record_descriptors;
        self.frozen_reference_descriptors = reference_descriptors;
        self.frozen_union_descriptors = union_descriptors;
    }
}

/// ADR-009 B5 (Dec 57): the owner-bound HYGIENIC member identity for a nominal
/// member (struct field / enum variant / associated const). A stable opaque
/// 128-bit identity minted from `member:{owner}:{member}` — NEVER the source
/// name string (a source spelling is not a member identity, R1/R3). Distinct
/// from the value-type identity space (the `member:` descriptor prefix).
fn member_identity(owner_name: &str, member_name: &str) -> FrozenTypeIdentity {
    FrozenTypeIdentity::from_canonical_descriptor(&format!("member:{owner_name}:{member_name}"))
}

/// ADR-009 B7: the internal-invariant message when a base-interned composite
/// identity carries its category but not its structural descriptor. The alias
/// fixpoint threads the descriptor in the SAME round it interns the identity
/// (write-once), so this is unreachable in practice — a named invariant, never a
/// partial descriptor.
fn composite_without_descriptor_invariant(category: &str) -> String {
    format!(
        "internal invariant: a {category} identity was frozen without its structural \
         composite descriptor"
    )
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
    /// ADR-009 B6 (Dec 63): the ordered structural descriptor for a `Callable`
    /// form (ordered params with name/type-identity/optionality/passing-mode +
    /// return identity). `None` for every non-callable category. The one-way
    /// SHA-256 identity drops parameter names; passing modes remain identity-
    /// significant through the canonical outer borrow wrapper. This preserved
    /// structure is what the freeze's widened composite memo carries so
    /// `payload_of` can reconstruct a `FrozenCallable` WITHOUT inverting the
    /// hash. Identity-insignificant: two callables that produce the same
    /// canonical descriptor share one identity regardless of their AST param
    /// names.
    pub(super) callable: Option<payloads::CallableDescriptor>,
    /// ADR-009 B7 (Dec 50/94): the ordered element identities for a `Tuple`
    /// form. `None` for every non-tuple category. Threaded through the freeze's
    /// widened composite memo (and the base per-category map) exactly like
    /// `callable`, so `payload_of` reconstructs a `FrozenTuple` WITHOUT
    /// inverting the one-way identity hash.
    pub(super) tuple: Option<payloads::TupleDescriptor>,
    /// ADR-009 B7: the normalized structural fields for a `Record` form
    /// (owner-bound member identity + value-type identity + optionality). `None`
    /// for every non-record category. The hygienic member identity is minted
    /// from the record's own identity, so it is stable and source-name-free
    /// (Dec 57).
    pub(super) record: Option<payloads::RecordDescriptor>,
    /// ADR-009 B7: the mutability + referent identity for a `Reference` form.
    /// `None` for every non-reference category.
    pub(super) reference: Option<payloads::ReferenceDescriptor>,
    /// ADR-009 B7: the deduped byte-sorted member identities for a `Union`
    /// form. `None` for every non-union category (and for a singleton union,
    /// which coalesces to its member — no `Union` descriptor exists).
    pub(super) union: Option<payloads::UnionDescriptor>,
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
            // ADR-009 B7: the ordered element identities are threaded on the
            // canonical (mirroring `callable`) so `payload_of` reconstructs the
            // `FrozenTuple` without inverting the one-way identity hash. Position
            // IS the tuple index — no separate index fact.
            let mut embedded = Vec::with_capacity(items.len());
            let mut elements = Vec::with_capacity(items.len());
            for item in items {
                let member = canonicalize_resolved(item, scope)?;
                embedded.push(identity_hex(member.identity));
                elements.push(member.identity);
            }
            let mut canonical = composite(
                format!("tuple:[{}]", embedded.join(",")),
                FrozenTypeCategory::Tuple,
            );
            canonical.tuple = Some(payloads::TupleDescriptor { elements });
            Ok(canonical)
        }
        TypeAnnotation::Object(fields) => canonical_record(fields, scope),
        TypeAnnotation::Function { params, returns, .. } => {
            // The canonical descriptor embeds each parameter's FULL annotation
            // identity (the `reference:&h` wrapper included, so a borrowed
            // parameter is identity-distinct from a by-value one) plus `?` —
            // names never appear (identity-insignificant). The B6 structural
            // descriptor is a PARALLEL derivation off the same walk: it factors
            // the ADR mode axis out of the borrow wrapper (`PassingMode`) and
            // records the parameter's VALUE-type identity (the referent when
            // borrowed), so `reflect()` can surface a `ParamDescriptor<Sig, I,
            // T, Mode>` without inverting the one-way identity hash.
            let mut embedded = Vec::with_capacity(params.len());
            let mut descriptors = Vec::with_capacity(params.len());
            for param in params {
                // ADR-009 B6 R2 (Dec 63): a callable's parameters are
                // heterogeneous signature-indexed descriptors, never a
                // homogeneous top-typed collection. A parameter spelled with the
                // compiler-internal `Any` top type (bare `Any` or `Array<Any>`)
                // erases the per-position type and is the named rejection —
                // parallel to the B3 witness-erasure posture (capital `Any`
                // refused; lowercase `any` is the enabled Erased leaf, reached
                // through `canonicalize_resolved` below untouched).
                if super::existential::annotation_erases_witness_to_any(&param.type_annotation) {
                    return Err(CALLABLE_PARAM_ERASED_TO_ANY_DIAGNOSTIC.to_string());
                }
                let member = canonicalize_resolved(&param.type_annotation, scope)?;
                embedded.push(format!(
                    "{}{}",
                    identity_hex(member.identity),
                    if param.optional { "?" } else { "" }
                ));
                // Passing mode has no second string field: its identity axis is
                // already encoded by the outermost borrow in `member.identity`.
                // The descriptor projects that same wrapper to an explicit mode;
                // its VALUE type is the referent when borrowed.
                let (mode, value_identity) = match &param.type_annotation {
                    TypeAnnotation::Borrow { mutable, inner } => {
                        let referent = canonicalize_resolved(inner, scope)?;
                        let mode = if *mutable {
                            PassingMode::ExclusiveBorrow
                        } else {
                            PassingMode::SharedBorrow
                        };
                        (mode, referent.identity)
                    }
                    _ => (PassingMode::Move, member.identity),
                };
                descriptors.push(payloads::ParamDescriptor {
                    name: param.name.clone(),
                    type_identity: value_identity,
                    optional: param.optional,
                    mode,
                });
            }
            let returns = canonicalize_resolved(returns, scope)?;
            let mut canonical = composite(
                format!(
                    "callable:({})->{}",
                    embedded.join(","),
                    identity_hex(returns.identity)
                ),
                FrozenTypeCategory::Callable,
            );
            canonical.callable = Some(payloads::CallableDescriptor {
                params: descriptors,
                returns: returns.identity,
            });
            Ok(canonical)
        }
        TypeAnnotation::Borrow { mutable, inner } => {
            let member = canonicalize_resolved(inner, scope)?;
            let mut canonical = composite(
                format!(
                    "reference:&{}{}",
                    if *mutable { "mut " } else { "" },
                    identity_hex(member.identity)
                ),
                FrozenTypeCategory::Reference,
            );
            // ADR-009 B7: mutability + referent identity threaded on the
            // canonical so `payload_of` reconstructs the `FrozenReference`.
            canonical.reference = Some(payloads::ReferenceDescriptor {
                mutable: *mutable,
                referent: member.identity,
            });
            Ok(canonical)
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
            // ADR-009 B7: the deduped, byte-sorted member identities threaded on
            // the canonical (in the SAME order the descriptor fixes) so
            // `payload_of` reconstructs the `FrozenUnion` without inverting the
            // one-way identity hash. Reconstructed from the sorted+deduped hex
            // embeddings, not the raw member list, so the payload member order
            // matches the identity's descriptor exactly.
            let union_members = embedded
                .iter()
                .map(|hex| {
                    identity_from_hex(hex).expect("union member hex is a well-formed identity")
                })
                .collect();
            let mut canonical = composite(
                format!("union:({})", embedded.join("|")),
                FrozenTypeCategory::Union,
            );
            canonical.union = Some(payloads::UnionDescriptor {
                members: union_members,
            });
            Ok(canonical)
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
        // ADR-009 B3 (S2): existential descriptor package
        // (`exists<W...> Descriptor<W...>`). The witnesses are canonicalized
        // POSITIONALLY (`witness:{index}` — de-Bruijn), so the package
        // identity is alpha-invariant (witness names don't matter) and
        // site-independent; the arity prefix makes it distinct per witness
        // count. Descriptor grammar: `exists:{arity}:{inner_hex}`. A witness
        // slot spelled with the compiler-internal `Any` top type erases the
        // witness — the named row-1 rejection, never a minted identity.
        TypeAnnotation::Existential { witnesses, inner } => {
            if witnesses.is_empty() {
                // The parser rejects `exists<>`; a fabricated empty AST would
                // otherwise mint a witness-less "existential" — reject it
                // rather than form a meaningless identity.
                return Err(
                    "existential descriptor package must bind at least one witness".to_string(),
                );
            }
            if super::existential::annotation_erases_witness_to_any(inner) {
                return Err(super::existential::WITNESS_ERASED_TO_ANY_DIAGNOSTIC.to_string());
            }
            let witness_index: HashMap<&str, usize> = witnesses
                .iter()
                .enumerate()
                .map(|(index, name)| (name.as_str(), index))
                .collect();
            let outer_resolve = scope.resolve;
            // Positional witness scope: a witness always shadows an outer name
            // (it is a bound de-Bruijn slot inside this package).
            let resolve = |name: &str| -> Option<(FrozenTypeIdentity, FrozenTypeCategory)> {
                if let Some(&index) = witness_index.get(name) {
                    return Some((
                        FrozenTypeIdentity::from_canonical_descriptor(&format!("witness:{index}")),
                        FrozenTypeCategory::Parameter,
                    ));
                }
                (outer_resolve)(name)
            };
            let witness_scope = LeafScope {
                resolve: &resolve,
                is_trait: scope.is_trait,
                applied_arity: scope.applied_arity,
            };
            let inner_canon = canonicalize_resolved(inner, &witness_scope)?;
            Ok(composite(
                format!(
                    "exists:{}:{}",
                    witnesses.len(),
                    identity_hex(inner_canon.identity)
                ),
                FrozenTypeCategory::Existential,
            ))
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
        callable: None,
        tuple: None,
        record: None,
        reference: None,
        union: None,
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
        callable: None,
        tuple: None,
        record: None,
        reference: None,
        union: None,
    })
}

fn canonical_record(
    fields: &[ObjectTypeField],
    scope: &LeafScope<'_>,
) -> Result<CanonicalType, String> {
    // ADR-009 B7: retain each field's name/optionality/VALUE-type identity so
    // the normalized `FrozenRecord` reconstructs without inverting the identity
    // hash. `entry.0` is the field NAME — identity-insignificant, hashed into a
    // hygienic member identity below, never surfaced as a runtime string
    // (Dec 57).
    let mut entries: Vec<(&str, bool, FrozenTypeIdentity)> = Vec::with_capacity(fields.len());
    for field in fields {
        let member = canonicalize_resolved(&field.type_annotation, scope)?;
        entries.push((field.name.as_str(), field.optional, member.identity));
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
        .map(|(name, optional, identity)| {
            format!(
                "{name}{}:{}",
                if *optional { "?" } else { "" },
                identity_hex(*identity)
            )
        })
        .collect();
    let mut canonical = composite(
        format!("record:{{{}}}", rendered.join(",")),
        FrozenTypeCategory::Record,
    );
    // The hygienic member identity is minted from the record's OWN frozen
    // identity + the field name (`member:record:{record_hex}:{name}`) — a stable
    // opaque identity, never the source-name string (Dec 57). Two records with
    // the same normalized shape mint the same identity AND the same member
    // identities, so the payload is a pure function of the frozen identity.
    let record_identity = canonical.identity;
    let record_fields = entries
        .iter()
        .map(
            |(name, optional, type_identity)| payloads::RecordFieldDescriptor {
                member: record_member_identity(record_identity, name),
                type_identity: *type_identity,
                optional: *optional,
                // ADR-009 E5 CKPT-3: preserve the plain field name as a
                // SPELL/REFLECT-ONLY freeze fact. `name` is NOT threaded into the
                // identity descriptor string (`rendered`, above) NOR into
                // `record_member_identity` — both stay byte-identical (the CKPT-0
                // binding invariant). It is read only when spelling the record back
                // (`reconstruct_type_annotation`) or reflecting its fields.
                name: name.to_string(),
            },
        )
        .collect();
    canonical.record = Some(payloads::RecordDescriptor {
        fields: record_fields,
    });
    Ok(canonical)
}

/// ADR-009 B7 (Dec 57): the owner-bound HYGIENIC member identity for one
/// structural-record field. A stable opaque 128-bit identity minted from
/// `member:record:{record_hex}:{name}` — the record's own frozen identity is the
/// owner (a structural record has no nominal owner name), NEVER the source-name
/// string. Distinct from the value-type identity space (the `member:` prefix
/// family, sibling of [`member_identity`]).
fn record_member_identity(record: FrozenTypeIdentity, field_name: &str) -> FrozenTypeIdentity {
    FrozenTypeIdentity::from_canonical_descriptor(&format!(
        "member:record:{}:{field_name}",
        identity_hex(record)
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
#[cfg(test)]
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
            callable: None,
            tuple: None,
            record: None,
            reference: None,
            union: None,
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
    let schema =
        shape_runtime::type_schema::lookup_schema_by_id_public(storage.schema_id as u32)
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
        return Err(
            "type_constructor received an unknown semantic type identity: the name is \
                    not a frozen nominal type constructor in this compilation unit"
                .to_string(),
        );
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

/// ADR-009 B5 (Dec 56): mint a `RepresentationAccess<T>` authority carrier for
/// a frozen type identity. The identity is re-validated through the freeze (an
/// unknown/INVALID identity cannot mint authority) so a malformed identity never
/// yields a usable capability. Only the two identity halves are stored — the
/// authority IS the identity binding, no name/kind text (Dec 56). Called ONLY by
/// the compiler-injected mint intrinsic in an annotation expand-hook scope.
pub(crate) fn build_representation_access_heap_value(
    identity: FrozenTypeIdentity,
    freeze: &FreezeOverlay,
) -> Result<HeapValue, String> {
    typed_slot_into_heap_value(build_representation_access_slot(identity, freeze)?)
}

/// The `KindedSlot` form of [`build_representation_access_heap_value`], for the
/// annotation expand-hook delivery path (`functions_annotations.rs`), which
/// binds the minted authority as a comptime module binding rather than an
/// intrinsic return value.
pub(crate) fn build_representation_access_slot(
    identity: FrozenTypeIdentity,
    freeze: &FreezeOverlay,
) -> Result<KindedSlot, String> {
    if identity == FrozenTypeIdentity::INVALID {
        return Err(
            "RepresentationAccess cannot be minted for an unknown semantic type identity"
                .to_string(),
        );
    }
    // Re-validate the identity is one the freeze actually issued: `category_of`
    // errors on an identity the freeze never minted, so a fabricated identity
    // cannot become authority.
    let _category = freeze.category_of(identity)?;
    Ok(typed_object_for_named_schema(
        COMPTIME_REPRESENTATION_ACCESS_SCHEMA,
        &[
            ("identity_high", KindedSlot::from_int(identity.high)),
            ("identity_low", KindedSlot::from_int(identity.low)),
        ],
    ))
}

/// Read the frozen type identity a `RepresentationAccess` carrier authorizes
/// (schema-name-checked). A slot that is not a genuine compiler-issued
/// `RepresentationAccess` — a bare int/bool, a `FrozenType`, an arbitrary object
/// — is the named R6 authority rejection.
fn representation_access_identity_from_ref(
    slot: &KindedSlot,
) -> Result<FrozenTypeIdentity, String> {
    let (schema, storage) = reserved_storage(
        slot,
        COMPTIME_REPRESENTATION_ACCESS_SCHEMA,
        "RepresentationAccess",
    )
    .map_err(|_| {
        "representation reflection requires explicit RepresentationAccess<T> authority: \
         reflect_repr's second argument must be the compiler-issued RepresentationAccess<T> \
         delivered to a declaration-attached annotation expand hook (author consent, Dec 56) — \
         an ordinary value cannot authorize it"
            .to_string()
    })?;
    identity_halves_of(&schema, storage, "identity_high", "identity_low")
}

/// ADR-009 B5 (Dec 56): `reflect_repr(TypeRef<T>, RepresentationAccess<T>)`. The
/// authority must be bound to the SAME frozen identity being reflected — a
/// capability minted for one type cannot decompose another (authority is not
/// ambient). Only then does the SAME payload builder as `reflect` answer the
/// complete `FrozenType` sum.
pub(crate) fn frozen_type_from_repr_ref(
    type_slot: &KindedSlot,
    access_slot: &KindedSlot,
    freeze: &FreezeOverlay,
) -> Result<HeapValue, String> {
    let identity = frozen_identity_from_ref(type_slot, "reflect_repr")?;
    let authorized = representation_access_identity_from_ref(access_slot)?;
    if authorized != identity {
        return Err(
            "RepresentationAccess<T> authorizes complete reflection over T only: this authority \
             is bound to a different type identity than the one being reflected — a capability \
             minted for one type cannot decompose another (Dec 56)"
                .to_string(),
        );
    }
    payloads::build_frozen_type_heap_value(identity, freeze)
}

/// Read one opaque argument's identity from a borrowed typed-object storage
/// (no owning `KindedSlot` is constructed, so nothing releases the borrowed
/// element on drop). Both `type_ref` and `const_arg` args carry the reserved
/// `TypeRef` schema — a foreign schema is a named rejection.
fn arg_identity_from_storage(storage: &TypedObjectStorage) -> Result<FrozenTypeIdentity, String> {
    let schema = shape_runtime::type_schema::lookup_schema_by_id_public(storage.schema_id as u32)
        .ok_or_else(|| {
        format!(
            "could not resolve apply-argument schema id {}",
            storage.schema_id
        )
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
            let high =
                TypedArray::get(ptr, i).ok_or("AppliedType arg identity read out of range")?;
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
        return Err(
            "apply expects its arguments as a checked array of type_ref / const_arg \
                    values"
                .to_string(),
        );
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
            let elem =
                TypedArray::get(array_ptr, i).ok_or("apply argument array read out of range")?;
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

#[cfg(test)]
mod tests;
