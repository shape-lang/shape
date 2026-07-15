//! ADR-009 §4.1 (ticket A1, slice S1): the canonical per-compilation-unit
//! semantic freeze. // ADR-009
//!
//! `SemanticFreeze` is the single frozen semantic type table built ONCE per
//! compilation unit at the registration-complete barrier
//! (`BytecodeCompiler::install_semantic_freeze`). On the graph-driven
//! pipeline the compilation unit is root + dependency modules and the
//! barrier runs at the graph entry point BEFORE Phase 1 dependency
//! compilation (`compile_with_graph_and_prelude`), so imported-module
//! comptime sites hold the handle too; `compile()` runs the barrier only
//! when it IS the entry point (single-module unit).
//! Slice S2 deleted the per-comptime-site `build_type_reflection_snapshot`
//! rebuilds (the S1 transitional scaffolding): every comptime site now
//! obtains this handle through [`BytecodeCompiler::comptime_freeze_overlay`]
//! — a site that cannot obtain one is a compile error
//! ([`NO_FREEZE_HANDLE_DIAGNOSTIC`], rejection-matrix row 3), never an
//! empty snapshot. Scoped generic parameters enter through a
//! [`FreezeOverlay`], never through a rebuild of the base index.
//!
//! ## Named freeze inputs (spec §4.1 no-two-derivations rule)
//!
//! Every fact enters the freeze from exactly ONE source; no fact is
//! re-derived from a second table:
//!
//! 1. **Struct shapes** — `BytecodeCompiler::struct_types` (field order) plus
//!    `BytecodeCompiler::struct_generic_info.runtime_field_types` (field
//!    annotations). These compiler tables are today the only source of
//!    ordered runtime struct-field annotations; per spec §4.1 ("promotes that
//!    data into the shared surface or documents it as freeze input") they are
//!    documented here as named freeze inputs. The freeze does not re-derive
//!    struct shapes from any other surface.
//! 2. **Type aliases** — alias names from `BytecodeCompiler::type_aliases`;
//!    the full target annotation from the type-inference environment's alias
//!    entry (`type_inference.env.lookup_type_alias`). Both stores are written
//!    by the same `Item::TypeAlias` registration from the one source
//!    declaration; the environment carries the structural annotation
//!    (composite targets like `type Pair = [int, string]`), the string table
//!    only a simple-name projection (named freeze input, same rationale).
//! 3. **Enums** — `BytecodeCompiler::type_tracker.schema_registry()`, the
//!    canonical schema registry.
//! 4. **Trait identities** (ticket B2, slice S2) —
//!    `compiler.type_inference.env` trait-def registry
//!    (`all_trait_defs()`), populated at the barrier by the B2 S1
//!    two-sub-pass predeclare walk. Read ONCE here; no per-site re-read.
//!    Traits are a DISTINCT identity kind (Dec 49): canonical
//!    `trait:{name}` descriptors in the SEPARATE `frozen_trait_ids` map —
//!    never interned into `FrozenTypeIndex.frozen_type_ids` (so
//!    `type_ref(TraitName)` keeps failing and `intern_identity`'s
//!    cross-category collision assertion never sees them), and with NO
//!    `FrozenTypeCategory::Trait` variant (Dec 50 rule 5).
//! 5. **Impl evidence** (ticket B2, slice S2) — the same env registry's
//!    trait-impl entries (`all_trait_impl_entries()`, default AND named
//!    impls), frozen as [`FrozenImplEvidence`] keyed
//!    `(trait_identity, type_identity)` with canonical
//!    `impl:{trait}:{type}:{impl_name_or_default}` descriptor identities.
//!    Ruled stance (B2): only direct (default + named) impls freeze as
//!    evidence; blanket-impl satisfaction and the legacy `implements`
//!    int→number widening rule do NOT silently become evidence — querying a
//!    pair the legacy path would have satisfied through them is a NAMED
//!    surface-and-stop diagnostic, never a silent `None`
//!    (considered-compromise log lands in defections.md, slice S6). An impl
//!    whose trait/target has no frozen identity is un-queryable by
//!    construction (no `trait_ref`/`type_ref` can name it) and is skipped;
//!    an impl whose unqualified trait name matches more than one frozen
//!    trait poisons the candidate pairs so the query surfaces-and-stops.
//! 6. **Unresolved-inference-variable detection** — the analyzer's canonical
//!    `\u{1}tyvar:` annotation encoding (`annotation_as_tyvar`), i.e. the
//!    exact vocabulary the `TypeInferenceEngine` substitution store uses.
//!    No parallel encoding is introduced.
//! 5. **Trait names** (ADR-009 A2, slice S5) — `BytecodeCompiler::known_traits`.
//!    Trait items predeclare their names alongside aliases/enums
//!    (`predeclare_item_semantic_freeze_inputs`) because full pass-1 trait
//!    registration (`register_item_functions`) runs AFTER the barrier. `dyn`
//!    bounds and trait intersections in checked type expressions resolve
//!    against this set. Input 1 additionally projects each struct's declared
//!    ordered generic PARAMETER KINDS (`struct_generic_info.type_params`
//!    mapped `TypeParam::Type`→`ParamKind::Type`, `TypeParam::Const`→
//!    `ParamKind::Const`) for applied-arity AND type-vs-const kind
//!    enforcement (ADR-009 B4); arity is the vector length, so there is one
//!    projection, not a separate arity table. Enum generic arity/kinds are
//!    not recoverable from the schema registry, so applied enum heads are
//!    arity/kind-unchecked in the A2 spelling path and a NAMED
//!    surface-and-stop under the B4 `param_kinds_of` query (never a guessed
//!    kind).
//!
//! ## Freeze-boundary rejection (Dec 52, rejection-matrix row 4)
//!
//! Freezing partial semantic state is a named-diagnostic error, never a
//! populated freeze: a struct field or alias whose annotation still carries
//! an unresolved inference variable rejects the whole freeze before any
//! comptime body could observe it. `SemanticFreeze` deliberately has NO
//! `Default` impl and no empty constructor — an "empty freeze" cannot exist.
//! The old `TypeReflectionSnapshot::default()` annotation-handler defect was
//! exactly the shape this forbids; S2 deleted it. S3 resolved the pre-pass
//! ordering by moving the barrier ahead of the speculative annotation
//! pre-passes in `compile()`: EVERY annotation-handler execution
//! (speculative or authoritative) consumes the real registration-complete
//! handle. The S2-era pre-pass reflection-rejection module is deleted;
//! exemption-by-suppression is a forbidden shape (S3 pre-pass freeze rule,
//! `functions_annotations.rs::s3_freeze_gate_tests`).
//!
//! ## Identity scheme
//!
//! The identity scheme (SHA-256 canonical descriptors, alias-fixpoint
//! transparency, primitive-synonym coalescing, collision assertion) is reused
//! byte-for-byte from `type_reflection`: the freeze calls the same interning
//! code (`rebuild_frozen_type_index`), it does not reimplement it. The S1
//! byte-identical parity oracle retired with the old builder's deletion;
//! `freeze_pins_identity_scheme_through_the_query_api` below and the 9-test
//! identity matrix in `type_reflection/tests.rs` are the ongoing tripwires.

use super::type_reflection::{
    FrozenTypeCategory, FrozenTypeIdentity, FrozenTypeIndex, canonicalize_type_annotation, payloads,
};
use crate::compiler::BytecodeCompiler;
use shape_ast::ast::{TypeAnnotation, TypeParam};
use shape_ast::error::{Result, ShapeError};
use shape_runtime::comptime_reflection::ParamKind;
use shape_runtime::type_system::{
    TypeVar, annotation_as_tyvar, annotation_contains_reserved_type_var_carrier,
};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

mod closed_semantic_type;
mod lexical_parameters;
mod projection;
mod specialization_overlay;
pub(crate) use closed_semantic_type::ClosedSemanticType;
use lexical_parameters::LexicalParameters;
pub(crate) use projection::{FrozenSemanticTypeProjection, annotation_has_lossy_unknown_sentinel};
pub(crate) use specialization_overlay::SpecializationTypeOverlay;

/// Rejection-matrix row 3 (ticket A1): a comptime site that cannot obtain
/// the per-compilation-unit freeze handle is a compile error — never an
/// empty snapshot, never a per-site rebuild.
pub(crate) const NO_FREEZE_HANDLE_DIAGNOSTIC: &str = "comptime site has no semantic freeze handle: the per-compilation-unit \
     semantic-freeze barrier (ADR-009 §4.1) must run before any comptime \
     site executes";

/// Named-diagnostic freeze failure (Dec 52 vocabulary). A freeze either
/// completes over fully resolved semantic state or rejects with one of these
/// — there is no partially populated freeze.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SemanticFreezeError {
    /// Rejection-matrix row 4: the named subject cannot be frozen because its
    /// type still contains an unresolved inference variable.
    UnresolvedInferenceVariable { subject: String },
    /// B2 S2 (Dec 52): two impl registrations collapse to the same canonical
    /// evidence slot (e.g. synonym targets `number`/`f64`) but carry
    /// DIFFERING facts — an ambiguously populated freeze cannot exist.
    ConflictingImplEvidence { subject: String },
}

impl SemanticFreezeError {
    pub(crate) fn diagnostic(&self) -> String {
        match self {
            Self::UnresolvedInferenceVariable { subject } => format!(
                "semantic freeze rejected: {subject} cannot be frozen because \
                 its type contains an unresolved inference variable"
            ),
            Self::ConflictingImplEvidence { subject } => format!(
                "semantic freeze rejected: conflicting trait-impl facts for \
                 {subject} — two registrations collapse to the same canonical \
                 evidence identity but carry different facts"
            ),
        }
    }
}

/// B2 S2 ruled stance (recorded in `docs/defections.md`): blanket-impl
/// satisfaction is not frozen evidence.
///
/// NOTE: this text is user-facing through the comptime diagnostics firewall
/// (`helpers.rs::sanitize_comptime_internal`) — it must stay free of jargon
/// markers (no "ADR-…", no "§", …) or the firewall replaces it wholesale
/// (pinned by `b2_user_facing_diagnostics_are_firewall_safe`).
pub(crate) const BLANKET_IMPL_NOT_EVIDENCE_DIAGNOSTIC: &str = "blanket-impl satisfaction is not frozen implementation evidence: the \
     queried pair has no direct (default or named) impl, and the trait has \
     blanket impls whose satisfaction the semantic freeze does not certify \
     — only direct implementations are compiler-issued evidence";

/// B2 S2 ruled stance (recorded in `docs/defections.md`): the legacy
/// `implements` int→number widening rule is not frozen evidence (E5 deletes
/// the legacy rule). Firewall-safe text — see the note on
/// [`BLANKET_IMPL_NOT_EVIDENCE_DIAGNOSTIC`].
pub(crate) const NUMERIC_WIDENING_NOT_EVIDENCE_DIAGNOSTIC: &str = "legacy numeric widening is not frozen implementation evidence: the \
     queried integer type has no direct impl for this trait — an impl \
     registered for `number` does not certify the integer family (the \
     widening rule belongs to the legacy `implements` builtin, which is \
     scheduled for deletion)";

/// B2 S2: an impl registered under an unqualified trait name that matches
/// more than one frozen trait def cannot be attributed — the query
/// surfaces-and-stops instead of guessing or silently missing.
pub(crate) const AMBIGUOUS_IMPL_EVIDENCE_DIAGNOSTIC: &str = "ambiguous trait-impl evidence: an impl registered under an unqualified \
     trait name matches more than one frozen trait definition";

/// ADR-009 B4 (Stage 2, Dec 54): a nominal head whose generic parameter
/// kinds are not recoverable from the freeze — today only user-declared enum
/// generics, whose arity/kinds the schema registry does not carry
/// (`semantic_freeze` named freeze input 3 gap). Rather than guess a kind,
/// the `param_kinds_of` query surfaces-and-stops with this named diagnostic;
/// `.apply(...)` on such a constructor cannot proceed. Builtin nominal
/// constructors (`Option`/`Result`/`Array`/collections/`Future`) and user
/// STRUCT generics carry frozen kinds and never reach this arm.
///
/// NOTE: user-facing through the comptime diagnostics firewall — kept free of
/// jargon markers (no "ADR-…", no "§", …).
pub(crate) const ENUM_HEAD_PARAM_KIND_UNRECOVERABLE_DIAGNOSTIC: &str = "generic parameter kinds are not recoverable for this nominal type: it is a \
     generic enum, whose declared parameters the semantic freeze does not \
     carry — arity and type-vs-const kind checking cannot proceed (only \
     builtin constructors and struct generics carry frozen parameter kinds)";

/// ADR-009 B4: `param_kinds_of` was asked for a non-nominal identity (a
/// primitive, tuple, callable, union, …). Only nominal type constructors
/// have generic parameter kinds; this is a named rejection, never `None`.
pub(crate) const NOT_A_TYPE_CONSTRUCTOR_DIAGNOSTIC: &str = "this type is not a nominal type constructor: only nominal types carry \
     generic parameter kinds";

/// ADR-009 B4 (Dec 54): project one declared generic parameter to its sealed
/// [`ParamKind`]. The ONE source of a constructor's param kinds — the AST
/// `TypeParam` variant, never re-derived from a second table.
pub(super) fn param_kind_of(param: &TypeParam) -> ParamKind {
    match param {
        TypeParam::Type { .. } => ParamKind::Type,
        TypeParam::Const { .. } => ParamKind::Const,
    }
}

/// B2 S1 ruled stance, landed in S5 (Dec 52 registration-complete ordering):
/// an implementation registered AFTER the semantic-freeze barrier — the
/// comptime-generated families (annotation/extend paths, From/TryFrom-derived
/// Into/TryInto, J-CT.2 `comptime impl` blocks) — is NOT frozen evidence.
/// Querying such a pair through `find_impl` is this named surface-and-stop;
/// it must never masquerade as the genuinely-unimplemented `None` answer.
///
/// NOTE: this text is user-facing through the comptime diagnostics firewall
/// (`helpers.rs::sanitize_comptime_internal`) — it must stay free of the
/// jargon markers (no "ADR-…", no "§", …) or the firewall replaces it
/// wholesale with the generic not-available sentence.
pub(crate) const POST_BARRIER_IMPL_NOT_EVIDENCE_DIAGNOSTIC: &str = "implementation registered after the semantic-freeze barrier is not \
     frozen implementation evidence: find_impl answers from \
     registration-complete barrier truth only, and this (type, trait) pair's \
     implementation was generated after the barrier (comptime-generated and \
     derived implementations are not barrier truth) — never a silent None";

/// ADR-009 (ticket B2, slice S2) named freeze input 5: ONE frozen trait-impl
/// fact (the `TraitImplEntry` shape read once at the barrier) — compiler-
/// issued evidence that `target_type` implements `trait_name`. `identity` is
/// the canonical impl-descriptor hash
/// (`impl:{trait}:{type}:{impl_name_or_default}`), so canonical trait and
/// implementation identities enter the SHA-256 fingerprint scheme (Dec 49).
#[derive(Debug, Clone)]
pub(crate) struct FrozenImplEvidence {
    /// Canonical impl identity (distinct per named impl).
    pub(crate) identity: FrozenTypeIdentity,
    /// The implemented trait's frozen identity (freeze input 4).
    pub(crate) trait_identity: FrozenTypeIdentity,
    /// The implementing type's frozen identity (value-type scheme).
    pub(crate) type_identity: FrozenTypeIdentity,
    /// Canonical (resolved) trait def name.
    pub(crate) trait_name: String,
    /// Target type name as registered (qualified where qualified).
    pub(crate) target_type: String,
    /// `impl Trait for Type as Name` selector (`None` = default impl).
    pub(crate) impl_name: Option<String>,
    /// Method names provided by this impl.
    pub(crate) method_names: Vec<String>,
    /// Associated type bindings: name → concrete type.
    pub(crate) associated_types: HashMap<String, TypeAnnotation>,
}

impl FrozenImplEvidence {
    /// Fact equality for canonical-slot coalescing (synonym targets like
    /// `number`/`f64`): method set + associated-type bindings.
    fn same_facts(&self, other: &Self) -> bool {
        let mut mine = self.method_names.clone();
        mine.sort();
        let mut theirs = other.method_names.clone();
        theirs.sort();
        mine == theirs && self.associated_types == other.associated_types
    }
}

/// All frozen evidence for one `(trait, type)` identity pair: at most one
/// default impl plus the named impls, each with a distinct canonical
/// identity. Constructed only inside [`SemanticFreeze::freeze`] — no
/// `Default`, no empty public constructor.
#[derive(Debug, Clone)]
pub(crate) struct FrozenImplEvidenceSet {
    default_impl: Option<FrozenImplEvidence>,
    named_impls: Vec<FrozenImplEvidence>,
}

impl FrozenImplEvidenceSet {
    /// Evidence for the default impl (`impl Trait for Type`), if any.
    pub(crate) fn default_impl(&self) -> Option<&FrozenImplEvidence> {
        self.default_impl.as_ref()
    }

    /// Evidence for named impls (`impl Trait for Type as Name`), sorted by
    /// impl name for deterministic enumeration.
    pub(crate) fn named_impls(&self) -> &[FrozenImplEvidence] {
        &self.named_impls
    }

    /// Insert one frozen fact. A second registration for the same canonical
    /// slot must carry identical facts (synonym-target coalescing keeps the
    /// deterministically-first entry); differing facts reject the freeze
    /// (Dec 52: never an ambiguously populated freeze).
    fn insert(
        &mut self,
        evidence: FrozenImplEvidence,
    ) -> std::result::Result<(), SemanticFreezeError> {
        let existing = match &evidence.impl_name {
            None => self.default_impl.as_ref(),
            Some(name) => self
                .named_impls
                .iter()
                .find(|entry| entry.impl_name.as_deref() == Some(name)),
        };
        if let Some(existing) = existing {
            if existing.same_facts(&evidence) {
                return Ok(());
            }
            return Err(SemanticFreezeError::ConflictingImplEvidence {
                subject: format!(
                    "impl {} for {} ({})",
                    evidence.trait_name,
                    evidence.target_type,
                    evidence.impl_name.as_deref().unwrap_or("default")
                ),
            });
        }
        match evidence.impl_name {
            None => self.default_impl = Some(evidence),
            Some(_) => self.named_impls.push(evidence),
        }
        Ok(())
    }
}

/// Resolution of an impl entry's AS-WRITTEN trait name against the frozen
/// trait-def names (freeze input 4). Impl entries register their trait name
/// as written while dep-module trait defs register qualified
/// (`qualify_module_item` qualifies the impl TARGET but not the trait name —
/// B2 S1), so resolution tries: exact name, then module-relative (the impl
/// lives in its target's module), then unique suffix.
enum TraitResolution {
    Resolved(String, FrozenTypeIdentity),
    Ambiguous(Vec<(String, FrozenTypeIdentity)>),
    Unknown,
}

fn resolve_frozen_trait(
    frozen_trait_ids: &HashMap<String, FrozenTypeIdentity>,
    as_written: &str,
    impl_target: Option<&str>,
) -> TraitResolution {
    if let Some(&identity) = frozen_trait_ids.get(as_written) {
        return TraitResolution::Resolved(as_written.to_string(), identity);
    }
    if let Some(target) = impl_target
        && let Some(split) = target.rfind("::")
    {
        let candidate = format!("{}::{}", &target[..split], as_written);
        if let Some(&identity) = frozen_trait_ids.get(&candidate) {
            return TraitResolution::Resolved(candidate, identity);
        }
    }
    let suffix = format!("::{as_written}");
    let mut matches: Vec<(String, FrozenTypeIdentity)> = frozen_trait_ids
        .iter()
        .filter(|(name, _)| name.ends_with(&suffix))
        .map(|(name, &identity)| (name.clone(), identity))
        .collect();
    matches.sort_by(|left, right| left.0.cmp(&right.0));
    match matches.len() {
        0 => TraitResolution::Unknown,
        1 => {
            let (name, identity) = matches.pop().expect("exactly one match");
            TraitResolution::Resolved(name, identity)
        }
        _ => TraitResolution::Ambiguous(matches),
    }
}

/// The per-compilation-unit frozen semantic type table (ADR-009 §4.1).
///
/// Internal storage is the [`FrozenTypeIndex`] (the reduced remainder of the
/// deleted per-site snapshot carrier) so the interning/identity code stays
/// single-sourced; the index lives only inside this freeze, never as a
/// reachable parallel carrier.
#[derive(Debug)]
pub(crate) struct SemanticFreeze {
    index: FrozenTypeIndex,
    /// Freeze input 4: canonical trait identities, keyed by registered trait
    /// def name. A DISTINCT identity kind (Dec 49) — deliberately a separate
    /// map from `index.frozen_type_ids` so `type_ref(TraitName)` keeps
    /// failing and no `FrozenTypeCategory` ever describes a trait (Dec 50
    /// rule 5).
    frozen_trait_ids: HashMap<String, FrozenTypeIdentity>,
    /// Freeze input 5: direct (default + named) impl evidence keyed
    /// `(trait_identity, type_identity)`.
    impl_evidence: HashMap<(FrozenTypeIdentity, FrozenTypeIdentity), FrozenImplEvidenceSet>,
    /// Candidate pairs poisoned by an ambiguously-attributable impl (value =
    /// the as-written trait name, for the diagnostic).
    ambiguous_impl_pairs: HashMap<(FrozenTypeIdentity, FrozenTypeIdentity), String>,
    /// Traits carrying at least one blanket impl (`impl<T: Bound> Trait for
    /// T`) — consulted for the ruled surface-and-stop stance, never as
    /// evidence.
    blanket_impl_traits: HashSet<FrozenTypeIdentity>,
}

impl SemanticFreeze {
    /// Build the freeze from the named freeze inputs (module doc). Called
    /// exactly once per compilation unit at the registration-complete
    /// barrier. Errors are freeze-boundary rejections (Dec 52): they fire
    /// before any user comptime body executes.
    pub(crate) fn freeze(
        compiler: &BytecodeCompiler,
    ) -> std::result::Result<Arc<Self>, SemanticFreezeError> {
        // The freeze barrier is the single construction point of the index:
        // `FrozenTypeIndex` deliberately has no `Default`/empty constructor
        // reachable elsewhere.
        let mut index = FrozenTypeIndex {
            struct_defs: HashMap::new(),
            enum_defs: HashMap::new(),
            alias_defs: HashMap::new(),
            trait_names: compiler.known_traits.clone(),
            struct_generic_param_kinds: HashMap::new(),
            struct_generic_param_names: HashMap::new(),
            frozen_type_ids: HashMap::new(),
            frozen_type_categories: HashMap::new(),
            frozen_primitive_payloads: HashMap::new(),
            frozen_callable_descriptors: HashMap::new(),
            frozen_nominal_descriptors: HashMap::new(),
            frozen_tuple_descriptors: HashMap::new(),
            frozen_record_descriptors: HashMap::new(),
            frozen_reference_descriptors: HashMap::new(),
            frozen_union_descriptors: HashMap::new(),
            generic_param_kinds: HashMap::new(),
        };

        // Named freeze input 1 (param-kind projection, S5 arity + B4 kinds):
        // the declared ordered generic parameter KINDS per struct from
        // `struct_generic_info.type_params` (arity = vector length — one
        // projection, not a second arity table). The rebuild below keys it by
        // frozen identity for alias-transparent applied-arity/kind
        // enforcement.
        for (name, info) in &compiler.struct_generic_info {
            index.struct_generic_param_kinds.insert(
                name.clone(),
                info.type_params.iter().map(param_kind_of).collect(),
            );
            // ADR-009 B5 (S2): the NAME projection of the same freeze input,
            // consumed by the applied-substitution path (bind param name →
            // applied argument identity before re-canonicalizing field types).
            index.struct_generic_param_names.insert(
                name.clone(),
                info.type_params
                    .iter()
                    .map(|param| param.name().to_string())
                    .collect(),
            );
        }

        // Named freeze input 1: struct shapes (`struct_types` field order +
        // `struct_generic_info.runtime_field_types` field annotations).
        let mut struct_names: Vec<&String> = compiler.struct_types.keys().collect();
        struct_names.sort();
        for name in struct_names {
            let (field_names, _span) = &compiler.struct_types[name];
            let field_types = compiler
                .struct_generic_info
                .get(name)
                .map(|info| &info.runtime_field_types);
            let mut ordered = Vec::with_capacity(field_names.len());
            for field_name in field_names {
                let Some(annotation) = field_types.and_then(|types| types.get(field_name)).cloned()
                else {
                    continue;
                };
                if annotation_has_unresolved_inference_variable(&annotation) {
                    return Err(SemanticFreezeError::UnresolvedInferenceVariable {
                        subject: format!("struct field {name}.{field_name}"),
                    });
                }
                ordered.push((field_name.clone(), annotation));
            }
            index.struct_defs.insert(name.clone(), ordered);
        }

        // Named freeze input 2: type aliases. Alias NAMES come from
        // `type_aliases`; the full target annotation comes from the
        // type-inference environment's alias entry — the compiler's
        // `Item::TypeAlias` registration writes both stores from the one
        // source declaration (single fact, one richer projection): the string
        // table keeps only a simple-name projection (a composite target like
        // `type Pair = [int, string]` is stored as a debug string there),
        // while the environment entry preserves the structural annotation the
        // A2 composite canonicalizer needs. Entries absent from the
        // environment (test-fabricated compilers) fall back to the
        // simple-name projection unchanged.
        for (alias, target) in &compiler.type_aliases {
            let annotation = compiler
                .type_inference
                .env
                .lookup_type_alias(alias)
                .map(|entry| entry.type_annotation.clone())
                .unwrap_or_else(|| TypeAnnotation::Basic(target.clone()));
            if annotation_has_unresolved_inference_variable(&annotation) {
                return Err(SemanticFreezeError::UnresolvedInferenceVariable {
                    subject: format!("type alias {alias}"),
                });
            }
            index.alias_defs.insert(alias.clone(), annotation);
        }

        // Named freeze input 3: enums from the canonical schema registry.
        let registry = compiler.type_tracker.schema_registry();
        for type_name in registry
            .type_names()
            .map(str::to_string)
            .collect::<Vec<_>>()
        {
            let Some(schema) = registry.get(&type_name) else {
                continue;
            };
            let Some(enum_info) = schema.get_enum_info() else {
                continue;
            };
            // ADR-009 B5 (Dec 57): the enriched enum freeze projection carries
            // the variant NAME (hashed into the owner-bound hygienic member
            // identity) AND its payload ARITY (`payload_fields`), the source of
            // `VariantDescriptor`. Full per-variant payload field TYPES are a
            // later B5 slice.
            index.enum_defs.insert(
                type_name,
                enum_info
                    .variants
                    .iter()
                    .map(|variant| {
                        super::type_reflection::FrozenEnumVariantDef {
                            name: variant.name.clone(),
                            payload_arity: variant.payload_fields,
                        }
                    })
                    .collect(),
            );
        }

        // The base freeze is module-scoped: function type parameters enter
        // ONLY through a `FreezeOverlay`, never through the base index.
        index.rebuild_frozen_type_index();

        // Named freeze input 4: trait identities — read ONCE from the
        // barrier-complete env registry (populated by the B2 S1 two-sub-pass
        // predeclare walk; `all_trait_defs()` is the single truth source).
        let env = &compiler.type_inference.env;
        let mut frozen_trait_ids = HashMap::new();
        for def in env.all_trait_defs() {
            frozen_trait_ids.insert(def.name.clone(), FrozenTypeIdentity::for_trait(&def.name));
        }
        // Mirror of `intern_identity`'s cross-category assertion for the
        // distinct trait kind: a trait identity may never collide with a
        // frozen value-type identity (disjoint descriptor spaces).
        for identity in frozen_trait_ids.values() {
            assert!(
                !index.frozen_type_categories.contains_key(identity),
                "canonical trait identity collision with a value-type identity"
            );
        }

        // Named freeze input 5: impl evidence — read ONCE from the same
        // barrier-complete registry (`all_trait_impl_entries()`, default AND
        // named impls), keyed `(trait_identity, type_identity)`. Sorted for
        // deterministic slot coalescing.
        let mut entries: Vec<_> = env.all_trait_impl_entries().collect();
        entries.sort_by(|left, right| {
            (&left.trait_name, &left.target_type, &left.impl_name).cmp(&(
                &right.trait_name,
                &right.target_type,
                &right.impl_name,
            ))
        });
        let mut impl_evidence: HashMap<
            (FrozenTypeIdentity, FrozenTypeIdentity),
            FrozenImplEvidenceSet,
        > = HashMap::new();
        let mut ambiguous_impl_pairs = HashMap::new();
        for entry in entries {
            let Some(type_identity) = index.frozen_type_id(&entry.target_type) else {
                // A target type with no frozen identity can never be named by
                // a `type_ref` (the only key into evidence): un-queryable by
                // construction — sound skip, not a masked miss.
                continue;
            };
            let (canonical_trait_name, trait_identity) = match resolve_frozen_trait(
                &frozen_trait_ids,
                &entry.trait_name,
                Some(&entry.target_type),
            ) {
                TraitResolution::Resolved(name, identity) => (name, identity),
                // A trait with no frozen def can never be named by a
                // `trait_ref`: un-queryable — sound skip.
                TraitResolution::Unknown => continue,
                // More than one frozen candidate: poison every candidate
                // pair so the QUERY surfaces-and-stops (named diagnostic)
                // instead of guessing or silently missing.
                TraitResolution::Ambiguous(candidates) => {
                    for (_, candidate) in candidates {
                        ambiguous_impl_pairs
                            .insert((candidate, type_identity), entry.trait_name.clone());
                    }
                    continue;
                }
            };
            let evidence = FrozenImplEvidence {
                identity: FrozenTypeIdentity::for_impl(
                    &canonical_trait_name,
                    &entry.target_type,
                    entry.impl_name.as_deref(),
                ),
                trait_identity,
                type_identity,
                trait_name: canonical_trait_name,
                target_type: entry.target_type.clone(),
                impl_name: entry.impl_name.clone(),
                method_names: entry.method_names.clone(),
                associated_types: entry.associated_types.clone(),
            };
            impl_evidence
                .entry((trait_identity, type_identity))
                .or_insert_with(|| FrozenImplEvidenceSet {
                    default_impl: None,
                    named_impls: Vec::new(),
                })
                .insert(evidence)?;
        }
        for set in impl_evidence.values_mut() {
            set.named_impls
                .sort_by(|left, right| left.impl_name.cmp(&right.impl_name));
        }

        // Ruled stance input: traits with blanket impls, so the query can
        // surface-and-stop instead of silently under-reporting (satisfaction
        // through a blanket impl is NOT frozen evidence).
        let mut blanket_impl_traits = HashSet::new();
        for blanket in env.all_blanket_impl_entries() {
            match resolve_frozen_trait(&frozen_trait_ids, &blanket.trait_name, None) {
                TraitResolution::Resolved(_, identity) => {
                    blanket_impl_traits.insert(identity);
                }
                // Conservative: every candidate surfaces.
                TraitResolution::Ambiguous(candidates) => {
                    for (_, identity) in candidates {
                        blanket_impl_traits.insert(identity);
                    }
                }
                TraitResolution::Unknown => {}
            }
        }

        Ok(Arc::new(Self {
            index,
            frozen_trait_ids,
            impl_evidence,
            ambiguous_impl_pairs,
            blanket_impl_traits,
        }))
    }

    /// Shared query API: canonical semantic identity for a resolvable name.
    pub(crate) fn identity_of(&self, name: &str) -> Option<FrozenTypeIdentity> {
        self.index.frozen_type_id(name)
    }

    /// Shared query API: semantic category for a frozen identity.
    pub(crate) fn category_of(
        &self,
        identity: FrozenTypeIdentity,
    ) -> std::result::Result<FrozenTypeCategory, String> {
        self.index.category_for_identity(identity)
    }

    /// Shared query API (ADR-009 B1 S2, spec §4.1 "as later tickets land —
    /// payload descriptors"): the payload descriptor for a frozen identity.
    /// Enabled categories (Primitive / Never / Erased) return complete typed
    /// payloads; a non-enabled category is the named R1 per-category
    /// rejection — never a partial descriptor.
    pub(crate) fn payload_of(
        &self,
        identity: FrozenTypeIdentity,
    ) -> std::result::Result<super::type_reflection::payloads::FrozenPayloadDescriptor, String>
    {
        self.index.payload_for_identity(identity)
    }

    /// Shared query API (ADR-009 B4, Dec 54): the ordered generic
    /// PARAMETER-KIND vector for a frozen nominal constructor identity —
    /// sibling of [`Self::category_of`] / [`Self::payload_of`], reading the
    /// ONE param-kind projection (no second arity table). Arity is the slice
    /// length; each element is the type-vs-const kind `.apply(...)` checks a
    /// supplied argument against.
    ///
    /// - `Ok(slice)` — a builtin constructor or user struct generic with
    ///   frozen kinds (an empty slice is a zero-parameter nominal — valid).
    /// - `Err(_)` — a NAMED surface-and-stop: a generic enum head whose kinds
    ///   are unrecoverable ([`ENUM_HEAD_PARAM_KIND_UNRECOVERABLE_DIAGNOSTIC`],
    ///   never a guessed kind), or a non-nominal identity
    ///   ([`NOT_A_TYPE_CONSTRUCTOR_DIAGNOSTIC`]). Never a silent `None`.
    pub(crate) fn param_kinds_of(
        &self,
        identity: FrozenTypeIdentity,
    ) -> std::result::Result<&[ParamKind], String> {
        if let Some(kinds) = self.index.generic_param_kinds.get(&identity) {
            return Ok(kinds.as_slice());
        }
        // No recorded kinds. A frozen NOMINAL identity with no kind vector is
        // the enum-head gap (freeze input 3 does not carry enum generics) —
        // surface-and-stop, never a guessed kind. Anything else is simply not
        // a type constructor.
        match self.index.frozen_type_categories.get(&identity).copied() {
            Some(FrozenTypeCategory::Nominal) => {
                Err(ENUM_HEAD_PARAM_KIND_UNRECOVERABLE_DIAGNOSTIC.to_string())
            }
            _ => Err(NOT_A_TYPE_CONSTRUCTOR_DIAGNOSTIC.to_string()),
        }
    }

    /// Shared query API (freeze input 4): canonical trait identity for a
    /// registered trait def name. A DISTINCT identity kind: value-type names
    /// resolve to `None` here, trait names resolve to `None` in
    /// [`Self::identity_of`].
    pub(crate) fn trait_identity_of(&self, name: &str) -> Option<FrozenTypeIdentity> {
        self.frozen_trait_ids.get(name).copied()
    }

    /// Shared query API (freeze input 4, reverse direction): true when
    /// `identity` is one of the trait identities this freeze issued. The
    /// opaque `TraitRef` carrier builders/decoders (`trait_evidence.rs`,
    /// slice S3) consult this so an identity the freeze never issued is a
    /// named rejection (R7 forged evidence), never a usable TraitRef.
    pub(crate) fn is_frozen_trait_identity(&self, identity: FrozenTypeIdentity) -> bool {
        self.frozen_trait_ids
            .values()
            .any(|&frozen| frozen == identity)
    }

    /// Shared query API (freeze input 5): frozen impl evidence for a
    /// `(trait, type)` identity pair.
    ///
    /// - `Ok(Some(_))` — direct (default and/or named) impl facts frozen at
    ///   the barrier.
    /// - `Ok(None)` — the pair is genuinely unimplemented in barrier truth.
    /// - `Err(_)` — named surface-and-stop (B2 ruled stance): no frozen
    ///   evidence exists but the legacy `implements` path would have
    ///   answered through a non-evidence rule (blanket-impl satisfaction,
    ///   int→number widening), or the impl's attribution was ambiguous.
    ///   Never a silent `None` for those cases.
    pub(crate) fn impl_evidence_of(
        &self,
        trait_identity: FrozenTypeIdentity,
        type_identity: FrozenTypeIdentity,
    ) -> std::result::Result<Option<&FrozenImplEvidenceSet>, String> {
        if let Some(set) = self.impl_evidence.get(&(trait_identity, type_identity)) {
            return Ok(Some(set));
        }
        if let Some(as_written) = self
            .ambiguous_impl_pairs
            .get(&(trait_identity, type_identity))
        {
            return Err(format!(
                "{AMBIGUOUS_IMPL_EVIDENCE_DIAGNOSTIC} (impl registered as '{as_written}')"
            ));
        }
        if self.is_integer_family_identity(type_identity)
            && let Some(number) = self.index.frozen_type_id("number")
            && self.impl_evidence.contains_key(&(trait_identity, number))
        {
            return Err(NUMERIC_WIDENING_NOT_EVIDENCE_DIAGNOSTIC.to_string());
        }
        if self.blanket_impl_traits.contains(&trait_identity) {
            return Err(BLANKET_IMPL_NOT_EVIDENCE_DIAGNOSTIC.to_string());
        }
        Ok(None)
    }

    /// Reverse direction of freeze input 4 (slice S5): the registered trait
    /// def names that froze to `identity`. Used by the S5 post-barrier
    /// ordering diagnostic (`trait_evidence.rs`) to name a Dec 52 violation
    /// — never to answer an evidence query.
    pub(crate) fn trait_names_for_identity(&self, identity: FrozenTypeIdentity) -> Vec<&str> {
        self.frozen_trait_ids
            .iter()
            .filter(|&(_, &frozen)| frozen == identity)
            .map(|(name, _)| name.as_str())
            .collect()
    }

    /// Reverse direction of the frozen TYPE identity map (slice S5): every
    /// name (including primitive synonyms) that froze to `identity`. Same
    /// diagnostic-only consumer contract as
    /// [`Self::trait_names_for_identity`].
    pub(crate) fn type_names_for_identity(&self, identity: FrozenTypeIdentity) -> Vec<&str> {
        self.index
            .frozen_type_ids
            .iter()
            .filter(|&(_, &frozen)| frozen == identity)
            .map(|(name, _)| name.as_str())
            .collect()
    }

    /// True when `identity` is a frozen integer-family primitive (the family
    /// the legacy `implements` widening rule promoted to `number`).
    fn is_integer_family_identity(&self, identity: FrozenTypeIdentity) -> bool {
        ["int", "i8", "i16", "i32", "u8", "u16", "u32", "u64"]
            .iter()
            .any(|name| self.index.frozen_type_id(name) == Some(identity))
    }

    /// The freeze's internal type index. Visible only inside
    /// `comptime_builtins` (the legacy `type_info` path reads nominal/alias/
    /// enum membership and struct field rows from the SAME frozen index —
    /// no second derivation; E5 deletes that consumer).
    pub(super) fn index(&self) -> &FrozenTypeIndex {
        &self.index
    }
}

/// Scoped generic-parameter overlay over an [`Arc<SemanticFreeze>`].
///
/// Carries `parameter:{owner}:{name}` identities for the enclosing function's
/// declared type parameters WITHOUT rebuilding the base index. Lookup order
/// reproduces the S1 builder's interning order byte-for-byte: a name already
/// frozen in the base (primitive, builtin nominal, user nominal, alias) wins
/// over a same-named parameter, exactly as `intern_identity`'s early return
/// did.
///
/// ADR-009 A2 (slice S2): the overlay additionally carries the site-interned
/// composite identities minted by [`FreezeOverlay::canonicalize_type`]
/// (tuples, records, callables, references, unions, erased trait objects,
/// applied generics). This is an extension of the ONE query API (spec §4.1)
/// — the memo is folded into the EXISTING [`FreezeOverlay::category_of`]
/// query (resolution order: scoped parameters → site-interned composites →
/// base), never a parallel lookup entry point and never a per-site rebuild
/// of the shared base index. The same `Arc<FreezeOverlay>` that performs the
/// comptime rewrite flows to the reflection intrinsics
/// (`execute_comptime_with_context` passes one handle to both), so composite
/// identities minted at rewrite time are classifiable at intrinsic time with
/// zero intrinsic-side changes.
///
/// Known semantic edge (inherited, unchanged): a module-level alias whose
/// target is a function type parameter resolved through the old per-site
/// rebuild's fixpoint; that shape is not valid Shape at module scope and is
/// not reproduced by the overlay.
#[derive(Debug)]
pub(crate) struct FreezeOverlay {
    base: Arc<SemanticFreeze>,
    lexical_parameters: LexicalParameters,
    exact_semantic_arguments: HashMap<TypeVar, ClosedSemanticType>,
    /// ADR-009 B3 (S2): hidden witnesses opened by
    /// [`FreezeOverlay::open_witnesses`] for one `comptime for some<W...>`
    /// site (`parameter:{some_site}:{witness}` identities). A DISTINCT binder
    /// layer from `parameters`: a declared type parameter defers to a
    /// same-named base identity (interning-order parity), but a freshly-opened
    /// hidden witness ALWAYS shadows the base — it is a new opaque type, so the
    /// witness layer is consulted first in every query below.
    witnesses: HashMap<String, FrozenTypeIdentity>,
    /// Site-interned composite identities (S2). Interior-mutable because the
    /// overlay is shared as `Arc<FreezeOverlay>` between the rewrite and the
    /// intrinsics; populated ONLY by [`FreezeOverlay::canonicalize_type`].
    ///
    /// ADR-009 B6 (Dec 63): the value is WIDENED from a bare category to
    /// [`CompositeMemoEntry`] so a `Callable` composite carries its ordered
    /// structural descriptor (param name/type-identity/optionality/passing-mode
    /// + return). Parameter names are identity-insignificant; passing modes are
    /// identity-significant through the canonical outer borrow wrapper and are
    /// also projected explicitly in the descriptor. Widening the memo value
    /// (rather than a parallel identity-keyed
    /// side-table) is the load-bearing B6 change — a second table would drift
    /// from this one. See `docs/defections.md` for the rejected side-table
    /// alternative.
    composites: Mutex<HashMap<FrozenTypeIdentity, CompositeMemoEntry>>,
}

/// ADR-009 B6 (Dec 63): the widened value of the site-interned composite memo.
/// Every composite carries its exhaustive semantic [`FrozenTypeCategory`]; a
/// `Callable` composite ALSO carries its ordered structural descriptor
/// (`callable`) so [`FreezeOverlay::payload_of`] can reconstruct a
/// `FrozenCallable` without inverting the identity hash. `callable` is `Some`
/// iff `category == Callable`.
///
/// ADR-009 B5 (Dec 55): an APPLIED nominal composite (`applied:h<…>`) ALSO
/// carries its refined head + ordered argument identities (`applied_nominal`)
/// so [`FreezeOverlay::payload_of`] can substitute the head's declared type
/// parameters before issuing descriptors — WITHOUT re-parsing the one-way
/// identity hash. Same widening discipline as `callable` (a widened memo value,
/// never a parallel identity-keyed side-table). `applied_nominal` is `Some` iff
/// the descriptor decomposes as a genuine `applied:` form.
///
/// ADR-009 B7 (Dec 50/94): a `Tuple` / `Record` / `Reference` / `Union`
/// composite ALSO carries its structural descriptor (`tuple` / `record` /
/// `reference` / `union`) so [`FreezeOverlay::payload_of`] reconstructs the
/// composite payload without inverting the identity hash — the same widening
/// discipline as `callable` (a widened memo value, never a parallel
/// identity-keyed side-table). Each is `Some` iff `category` is the matching
/// composite category.
#[derive(Debug, Clone)]
struct CompositeMemoEntry {
    category: FrozenTypeCategory,
    callable: Option<payloads::CallableDescriptor>,
    applied_nominal: Option<super::type_reflection::RefinedApplication>,
    tuple: Option<payloads::TupleDescriptor>,
    record: Option<payloads::RecordDescriptor>,
    reference: Option<payloads::ReferenceDescriptor>,
    union: Option<payloads::UnionDescriptor>,
}

/// ADR-009 B7: the internal-invariant message when a site-interned composite
/// memo entry carries its category but not its structural descriptor. The
/// canonicalizer threads the descriptor onto the same `CompositeMemoEntry` it
/// categorizes, so this is unreachable in practice — a named invariant, never a
/// partial descriptor.
fn composite_memo_invariant(category: &str) -> String {
    format!(
        "internal invariant: a {category} composite was memoized without its structural \
         descriptor"
    )
}

impl FreezeOverlay {
    /// Overlay `type_params` scoped to `parameter_owner` (the enclosing
    /// function name; module scope uses `"<module>"`, matching the S1
    /// builder's default owner).
    pub(crate) fn new(
        base: Arc<SemanticFreeze>,
        parameter_owner: &str,
        type_params: &[String],
    ) -> Self {
        Self::new_with_parameter_scopes(base, [(parameter_owner.to_string(), type_params.to_vec())])
    }

    /// Construct an overlay over ordered lexical Parameter scopes. Outer
    /// scopes come first; later (inner) scopes shadow the same source name.
    /// Base identities retain precedence over every Parameter spelling.
    fn new_with_parameter_scopes(
        base: Arc<SemanticFreeze>,
        parameter_scopes: impl IntoIterator<Item = (String, Vec<String>)>,
    ) -> Self {
        let lexical_parameters = LexicalParameters::new(&base, parameter_scopes);
        Self {
            base,
            lexical_parameters,
            exact_semantic_arguments: HashMap::new(),
            witnesses: HashMap::new(),
            composites: Mutex::new(HashMap::new()),
        }
    }

    fn with_exact_semantic_arguments(
        mut self,
        exact_semantic_arguments: HashMap<TypeVar, ClosedSemanticType>,
    ) -> Self {
        self.exact_semantic_arguments = exact_semantic_arguments;
        self
    }

    pub(crate) fn exact_semantic_argument(
        &self,
        declared: &TypeVar,
    ) -> Option<&ClosedSemanticType> {
        self.exact_semantic_arguments.get(declared)
    }

    /// Freeze-issued lexical Parameter context in outer-to-inner scope order.
    ///
    /// The identities are semantic cache material only; callers never rebuild
    /// them from owner/name strings. Shadowed outer identities are retained
    /// conservatively, while base-shadowed spellings are absent because they
    /// never classified as Parameters in this overlay.
    pub(crate) fn lexical_parameter_identities(&self) -> &[FrozenTypeIdentity] {
        self.lexical_parameters.ordered_identities()
    }

    /// Shared query API: opened hidden witnesses shadow first (S2 — a witness
    /// is a fresh opaque type), then base identities (interning-order parity),
    /// then this overlay's scoped declared parameters.
    pub(crate) fn identity_of(&self, name: &str) -> Option<FrozenTypeIdentity> {
        if let Some(&identity) = self.witnesses.get(name) {
            return Some(identity);
        }
        self.base
            .identity_of(name)
            .or_else(|| self.lexical_parameters.identity_of(name))
    }

    /// Shared query API: overlay parameters classify as
    /// [`FrozenTypeCategory::Parameter`]; site-interned composites (S2)
    /// answer their structural category; everything else defers to the base.
    /// One query, three layers — no second lookup entry point exists.
    pub(crate) fn category_of(
        &self,
        identity: FrozenTypeIdentity,
    ) -> std::result::Result<FrozenTypeCategory, String> {
        if self.witnesses.values().any(|&witness| witness == identity)
            || self.lexical_parameters.contains_identity(identity)
        {
            return Ok(FrozenTypeCategory::Parameter);
        }
        if let Some(category) = self
            .composites
            .lock()
            .expect("freeze-overlay composite memo lock poisoned")
            .get(&identity)
            .map(|entry| entry.category)
        {
            return Ok(category);
        }
        self.base.category_of(identity)
    }

    /// Shared query API (ADR-009 B1 S2): the payload half resolves the SAME
    /// three layers as [`FreezeOverlay::category_of`] — one query, three
    /// layers (spec §4.1). Overlay-scoped generic parameters (and opened
    /// existential witnesses) classify as [`FrozenTypeCategory::Parameter`] and,
    /// since ADR-009 B7 Slice 2, answer a COMPLETE `TypeParamDescriptor` off
    /// their stable base-fn-scoped `parameter:{owner}:{name}` identity (the
    /// public A3 path, Dec 50/94) — never an inference hole, never a partial
    /// descriptor. Site-interned
    /// composites (A2) answer by their memoized structural category: a
    /// base-frozen identity (union coalescing memoizes leaf members, e.g.
    /// `int | i64` → the `int` leaf) answers its complete payload from the
    /// base; an Erased composite is a `dyn`/trait-intersection bound set
    /// whose typed bound elements land with ticket B2 — the named
    /// bounded-erased rejection, never an empty (partial) bound set; every
    /// pending composite category is its named R1 rejection. Everything
    /// else defers to the base freeze.
    pub(crate) fn payload_of(
        &self,
        identity: FrozenTypeIdentity,
    ) -> std::result::Result<super::type_reflection::payloads::FrozenPayloadDescriptor, String>
    {
        if self.witnesses.values().any(|&witness| witness == identity)
            || self.lexical_parameters.contains_identity(identity)
        {
            // ADR-009 B7 Slice 2 (Dec 50/94): a scoped generic parameter (or an
            // opened existential witness — a fresh opaque parameter-like type)
            // answers a COMPLETE `TypeParamDescriptor`. Its `identity` is the
            // queried `parameter:{owner}:{name}` identity itself — stable and
            // base-fn-scoped under monomorphization (Decision 52), NEVER an
            // inference hole. The declared bound set is provably empty today
            // (`FrozenParameterBound` is uninhabited until ticket B2 lands the
            // trait-reference descriptors) — the honest "bounds where
            // representable" form, mirroring `FrozenErased`, never a partial
            // descriptor.
            return Ok(payloads::FrozenPayloadDescriptor::Parameter(
                payloads::TypeParamDescriptor {
                    identity,
                    bounds: Vec::new(),
                },
            ));
        }
        let composite_entry = self
            .composites
            .lock()
            .expect("freeze-overlay composite memo lock poisoned")
            .get(&identity)
            .cloned();
        if let Some(entry) = composite_entry {
            let category = entry.category;
            // A memoized identity the base ALSO froze (union coalescing onto
            // a base leaf, or a spelled composite that an alias fixpoint
            // interned into the base) answers from the base index — same
            // payload/rejection, one derivation.
            if self.base.category_of(identity).is_ok() {
                return self.base.payload_of(identity);
            }
            return match category {
                // Only reachable via coalescing onto a base leaf, which the
                // base-frozen arm above already answered.
                FrozenTypeCategory::Primitive | FrozenTypeCategory::Never => Err(
                    "internal invariant: a base-leaf payload category was site-interned \
                     without a base-frozen identity"
                        .to_string(),
                ),
                // Site-interned Erased composites are `dyn`/trait-intersection
                // bound sets — non-empty by construction (`canonical_erased_bounds`
                // rejects the empty set); unrepresentable until B2.
                FrozenTypeCategory::Erased => Err(payloads::bounded_erased_payload_rejection()),
                // ADR-009 B6 (Dec 63): a site-interned callable answers its
                // complete signature descriptor from the widened memo — never a
                // partial descriptor. The entry carries the structure iff the
                // category is Callable (canonicalizer invariant).
                FrozenTypeCategory::Callable => entry
                    .callable
                    .map(payloads::FrozenPayloadDescriptor::Callable)
                    .ok_or_else(|| {
                        "internal invariant: a Callable composite was memoized without its \
                         FrozenCallable structural descriptor"
                            .to_string()
                    }),
                // ADR-009 B5 (Dec 55, S2): a site-interned Nominal composite is
                // an APPLIED generic form (`applied:h<…>`) not interned into the
                // base (a base user struct/enum is answered by the base arm
                // above). Generic substitution PRECEDES descriptor issuance: the
                // refined head + argument identities bind the head's declared
                // parameters, and the field annotations re-canonicalize through
                // the ONE canonicalizer under that binding. A head with no
                // frozen struct field annotations to substitute (a builtin/enum
                // applied form) has nothing to substitute into and stays the
                // named applied-substitution-pending rejection — never a
                // descriptor off the un-substituted form.
                FrozenTypeCategory::Nominal => entry
                    .applied_nominal
                    .as_ref()
                    .and_then(|applied| {
                        self.base
                            .index()
                            .substituted_applied_nominal(applied.head_identity, &applied.arg_identities)
                    })
                    .map(payloads::FrozenPayloadDescriptor::Nominal)
                    .ok_or_else(payloads::applied_nominal_pending_rejection),
                // ADR-009 B7 (Dec 50/94): a site-interned composite answers its
                // complete structural descriptor from the widened memo — never a
                // partial descriptor. The entry carries the descriptor iff the
                // category matches (canonicalizer invariant).
                FrozenTypeCategory::Tuple => entry
                    .tuple
                    .map(payloads::FrozenPayloadDescriptor::Tuple)
                    .ok_or_else(|| composite_memo_invariant("Tuple")),
                FrozenTypeCategory::Record => entry
                    .record
                    .map(payloads::FrozenPayloadDescriptor::Record)
                    .ok_or_else(|| composite_memo_invariant("Record")),
                FrozenTypeCategory::Reference => entry
                    .reference
                    .map(payloads::FrozenPayloadDescriptor::Reference)
                    .ok_or_else(|| composite_memo_invariant("Reference")),
                FrozenTypeCategory::Union => entry
                    .union
                    .map(payloads::FrozenPayloadDescriptor::Union)
                    .ok_or_else(|| composite_memo_invariant("Union")),
                pending @ (FrozenTypeCategory::Parameter
                // ADR-009 B3 (S2): a site-interned existential descriptor
                // package. Its iteration payload (the opened witness element
                // descriptors) lands with slice S3 — until then it is the
                // named R1 per-category rejection, never a partial descriptor.
                | FrozenTypeCategory::Existential) => {
                    Err(payloads::pending_payload_rejection(pending))
                }
            };
        }
        self.base.payload_of(identity)
    }

    /// Shared query API (ADR-009 B4, Dec 54): the ordered generic
    /// parameter-kind vector for a frozen nominal constructor identity.
    /// Scoped generic parameters are never nominal type constructors (a
    /// `parameter:{owner}:{name}` leaf has no declared generics), and
    /// site-interned composites are never constructor heads, so the overlay
    /// defers to the base freeze unconditionally — one query, one source.
    pub(crate) fn param_kinds_of(
        &self,
        identity: FrozenTypeIdentity,
    ) -> std::result::Result<&[ParamKind], String> {
        self.base.param_kinds_of(identity)
    }

    /// Shared query API passthrough (freeze input 4): trait identities are a
    /// DISTINCT identity kind — scoped generic parameters never shadow them,
    /// so the overlay defers to the base freeze unconditionally.
    pub(crate) fn trait_identity_of(&self, name: &str) -> Option<FrozenTypeIdentity> {
        self.base.trait_identity_of(name)
    }

    /// Shared query API passthrough (freeze input 4, reverse direction):
    /// scoped generic parameters are never traits, so the overlay defers to
    /// the base freeze unconditionally.
    pub(crate) fn is_frozen_trait_identity(&self, identity: FrozenTypeIdentity) -> bool {
        self.base.is_frozen_trait_identity(identity)
    }

    /// Shared query API passthrough (freeze input 5): impl evidence lives
    /// only in the base freeze (an overlay parameter is never an
    /// implementing type in barrier truth).
    pub(crate) fn impl_evidence_of(
        &self,
        trait_identity: FrozenTypeIdentity,
        type_identity: FrozenTypeIdentity,
    ) -> std::result::Result<Option<&FrozenImplEvidenceSet>, String> {
        self.base.impl_evidence_of(trait_identity, type_identity)
    }

    /// Shared query API passthrough (S5, reverse direction of freeze input
    /// 4): defers to the base freeze unconditionally — scoped generic
    /// parameters are never traits.
    pub(crate) fn trait_names_for_identity(&self, identity: FrozenTypeIdentity) -> Vec<&str> {
        self.base.trait_names_for_identity(identity)
    }

    /// Shared query API passthrough (S5, reverse TYPE-identity direction):
    /// defers to the base freeze — an overlay parameter identity has no
    /// registered impl-target name in barrier truth, so the base map is the
    /// complete diagnostic-name source.
    pub(crate) fn type_names_for_identity(&self, identity: FrozenTypeIdentity) -> Vec<&str> {
        self.base.type_names_for_identity(identity)
    }

    /// The shared base freeze this overlay scopes (no rebuild happened).
    pub(crate) fn base(&self) -> &Arc<SemanticFreeze> {
        &self.base
    }

    /// True when `name` is one of this overlay's scoped generic parameters
    /// (i.e. it resolves to [`FrozenTypeCategory::Parameter`] here and is
    /// not shadowed by a base identity).
    pub(crate) fn is_scoped_parameter(&self, name: &str) -> bool {
        self.witnesses.contains_key(name) || self.lexical_parameters.contains_name(name)
    }

    /// ADR-009 B3 (slice S2): open an existential descriptor package's hidden
    /// witnesses for ONE `comptime for some<W...>` iteration site. Fresh
    /// `parameter:{some_site}:{witness}` identities are scoped over the SAME
    /// shared base freeze — modeled on [`Self::new`]'s scoped-parameter
    /// mechanism and the specialization type-param overlay
    /// ([`BytecodeCompiler::comptime_freeze_overlay`]); this is an extension
    /// of the ONE freeze query surface, never a parallel iterator or a second
    /// reflection protocol.
    ///
    /// Two distinct sites never share a witness identity (the `some_site`
    /// discriminates), so a hidden witness cannot escape its opening scope by
    /// aliasing another site's. Unlike a declared type parameter
    /// ([`Self::new`] defers to a same-named base identity), a hidden witness
    /// ALWAYS shadows: it is a freshly-opened type, so a base type of the
    /// same spelling never captures it. Any enclosing-scope generic
    /// parameters already carried by this overlay are preserved.
    pub(crate) fn open_witnesses(&self, some_site: &str, witnesses: &[String]) -> Self {
        let mut opened = self.witnesses.clone();
        for name in witnesses {
            let identity = FrozenTypeIdentity::from_canonical_descriptor(&format!(
                "parameter:{some_site}:{name}"
            ));
            opened.insert(name.clone(), identity);
        }
        Self {
            base: Arc::clone(&self.base),
            lexical_parameters: self.lexical_parameters.clone(),
            exact_semantic_arguments: self.exact_semantic_arguments.clone(),
            witnesses: opened,
            composites: Mutex::new(HashMap::new()),
        }
    }
}

/// Freeze-boundary predicate for rejection-matrix row 4: true when the
/// annotation structurally contains any reserved inference-variable carrier,
/// authenticated or tampered. The exhaustive walk is runtime-owned beside
/// carrier issuance and recovery; this VM seam only names freeze semantics.
///
/// `pub(super)`: the A2 composite canonicalizer (`type_reflection`) reuses
/// this exact predicate for its inference-hole rejection — one detector, no
/// second derivation.
pub(super) fn annotation_has_unresolved_inference_variable(annotation: &TypeAnnotation) -> bool {
    annotation_contains_reserved_type_var_carrier(annotation)
}

impl BytecodeCompiler {
    /// ADR-009 §4.1 registration-complete semantic-freeze barrier: install
    /// the single per-compilation-unit freeze. Runs exactly once per
    /// compilation unit — from the graph entry point before Phase 1 when
    /// compiling with a module graph, else from `compile()` (which consumes
    /// the compiler); a second install is an internal-invariant compile
    /// error, and a freeze-boundary rejection surfaces as a compile error
    /// BEFORE any comptime site could execute (Dec 52).
    pub(crate) fn install_semantic_freeze(&mut self) -> Result<()> {
        if self.semantic_freeze.is_some() {
            return Err(ShapeError::TypeError(
                "semantic freeze already installed: the freeze barrier runs \
                 exactly once per compilation unit"
                    .to_string(),
            ));
        }
        let freeze = SemanticFreeze::freeze(self)
            .map_err(|error| ShapeError::TypeError(error.diagnostic()))?;
        self.semantic_freeze = Some(freeze);
        // ADR-009 (ticket B2, slice S1) test oracle: snapshot the analyzer
        // env's trait/impl truth exactly at the barrier so tests can assert
        // registration-completeness THROUGH the real entry points. Test
        // instrumentation only — superseded when S2 freezes trait identities
        // and impl evidence as named freeze inputs (then the freeze query API
        // itself becomes the observation point).
        #[cfg(test)]
        barrier_env_truth_for_tests::record(
            self.type_inference
                .env
                .all_trait_defs()
                .map(|t| t.name.clone())
                .collect(),
            self.type_inference.env.trait_impl_keys(),
        );
        Ok(())
    }

    /// ADR-009 §4.1 (slice S2): the freeze handle a comptime site consumes.
    ///
    /// Returns the installed per-compilation-unit freeze, scoped by a
    /// [`FreezeOverlay`] over the enclosing function's declared type
    /// parameters when `current_function` is generic. A closure-aware
    /// specialization that explicitly splices caller AST may carry ordered
    /// outer lexical Parameter scopes as well; ordinary recursive compilation
    /// remains isolated. Module scope carries no parameter scope. This replaces
    /// the deleted per-site `build_type_reflection_snapshot` rebuild: the base
    /// index is shared (`Arc`), never rebuilt.
    ///
    /// A comptime site reached without an installed freeze is a compile
    /// error (rejection-matrix row 3, [`NO_FREEZE_HANDLE_DIAGNOSTIC`]) —
    /// never an empty snapshot.
    pub(crate) fn comptime_freeze_overlay(&self) -> Result<Arc<FreezeOverlay>> {
        let Some(freeze) = self.semantic_freeze.as_ref() else {
            return Err(ShapeError::TypeError(
                NO_FREEZE_HANDLE_DIAGNOSTIC.to_string(),
            ));
        };
        let mut parameter_scopes: Vec<(String, Vec<String>)> = Vec::new();
        if let Some(function) = self
            .current_function
            .and_then(|index| self.program.functions.get(index))
            && let Some(definition) = self.function_defs.get(&function.name)
            && let Some(parameters) = &definition.type_params
        {
            parameter_scopes.push((
                function.name.clone(),
                parameters
                    .iter()
                    .map(|parameter| parameter.name().to_string())
                    .collect(),
            ));
        }
        // ADR-009 A3 — specialization overlay: while a monomorphized body
        // compiles, the registered def carries `type_params = None`
        // (substitution strips them), so the discovery above finds nothing.
        // The overlay set around `compile_function` in
        // `monomorphization/cache.rs` re-supplies the BASE generic function's
        // declared type-param names, with the owner scoped to the BASE name
        // (never the mono key) so Parameter identities are declaration-stable
        // across instantiations (ADR-009 §Semantic Freeze, Decision 52
        // pre-substitution identities). The stack itself composes outer
        // lexical scopes only through its explicit closure-inline entry point;
        // an ordinary nested specialization exposes only its own scope.
        let specialization = self.specialization_type_overlays.current();
        if let Some(specialization) = specialization.as_ref() {
            parameter_scopes.extend(
                specialization
                    .parameter_scopes()
                    .map(|(owner, names)| (owner.to_string(), names.to_vec())),
            );
        }
        let mut overlay =
            FreezeOverlay::new_with_parameter_scopes(Arc::clone(freeze), parameter_scopes);
        if let Some(specialization) = specialization
            && specialization.has_exact_arguments()
        {
            overlay =
                overlay.with_exact_semantic_arguments(specialization.exact_arguments().clone());
        }
        Ok(Arc::new(overlay))
    }
}

/// Test-only fabricator (S2): a REAL freeze over a fabricated compiler,
/// wrapped in a module-scope overlay. This replaces the deleted
/// `TypeReflectionSnapshot::default()` + field-poking test constructions —
/// tests go through the same single freeze barrier as production code.
#[cfg(test)]
pub(crate) fn overlay_for_tests(compiler: &BytecodeCompiler) -> Arc<FreezeOverlay> {
    let freeze = SemanticFreeze::freeze(compiler).expect("test compiler state must freeze");
    Arc::new(FreezeOverlay::new(freeze, "<module>", &[]))
}

/// ADR-009 (ticket B2, slice S1) test oracle: the analyzer env's trait/impl
/// truth as observed AT `install_semantic_freeze` time (the registration-
/// complete barrier), captured per test thread. Both entry points
/// (`compile()` and `compile_with_graph_and_prelude`) consume the compiler,
/// so the barrier-time state is otherwise unobservable from a test. This is
/// cfg(test) instrumentation over the single truth source (the env
/// registry), not a parallel table; S2's trait-identity/impl-evidence freeze
/// inputs supersede it as the observation surface.
#[cfg(test)]
pub(crate) mod barrier_env_truth_for_tests {
    use std::cell::RefCell;
    use std::collections::HashSet;

    #[derive(Debug, Clone)]
    pub(crate) struct BarrierEnvTruth {
        /// Trait names registered in `type_inference.env` at the barrier.
        pub(crate) trait_names: HashSet<String>,
        /// `env.trait_impl_keys()` (legacy 2-part + canonical 3-part keys)
        /// at the barrier.
        pub(crate) trait_impl_keys: HashSet<String>,
    }

    thread_local! {
        static ENV_TRUTH_AT_BARRIER: RefCell<Option<BarrierEnvTruth>> =
            const { RefCell::new(None) };
    }

    pub(crate) fn record(trait_names: HashSet<String>, trait_impl_keys: HashSet<String>) {
        ENV_TRUTH_AT_BARRIER.with(|cell| {
            *cell.borrow_mut() = Some(BarrierEnvTruth {
                trait_names,
                trait_impl_keys,
            });
        });
    }

    /// Take the truth captured by the most recent barrier on this thread.
    pub(crate) fn take() -> Option<BarrierEnvTruth> {
        ENV_TRUTH_AT_BARRIER.with(|cell| cell.borrow_mut().take())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_runtime::type_schema::EnumVariantInfo;
    use shape_runtime::type_system::{TypeVarGen, tyvar_to_annotation};

    fn compiler_with_module_scope_types() -> BytecodeCompiler {
        let mut compiler = BytecodeCompiler::new();
        // Structs (named freeze input 1): field order + field annotations.
        compiler.struct_types.insert(
            "Point".to_string(),
            (
                vec!["x".to_string(), "y".to_string()],
                shape_ast::ast::Span::DUMMY,
            ),
        );
        compiler.struct_generic_info.insert(
            "Point".to_string(),
            crate::compiler::StructGenericInfo {
                type_params: Vec::new(),
                runtime_field_types: [
                    ("x".to_string(), TypeAnnotation::Basic("int".to_string())),
                    ("y".to_string(), TypeAnnotation::Basic("number".to_string())),
                ]
                .into_iter()
                .collect(),
            },
        );
        compiler.struct_types.insert(
            "Alpha".to_string(),
            (Vec::new(), shape_ast::ast::Span::DUMMY),
        );
        // Aliases (named freeze input 2), including a two-hop chain.
        compiler
            .type_aliases
            .insert("UserId".to_string(), "int".to_string());
        compiler
            .type_aliases
            .insert("First".to_string(), "Second".to_string());
        compiler
            .type_aliases
            .insert("Second".to_string(), "int".to_string());
        // Enum (named freeze input 3): canonical schema registry.
        compiler
            .type_tracker
            .schema_registry_mut()
            .register_enum_scoped(
                "Color",
                vec![
                    EnumVariantInfo::new("Red", 0, 0),
                    EnumVariantInfo::new("Green", 1, 0),
                    EnumVariantInfo::new("Blue", 2, 1),
                ],
            );
        compiler
    }

    /// The S1 parity oracle (byte-identical tables vs the old per-site
    /// builder) retired with the builder's S2 deletion; the identity scheme
    /// stays pinned through the shared query API — structs, enums, aliases,
    /// primitive synonyms, builtin nominals — plus the ongoing 9-test
    /// identity matrix in `type_reflection/tests.rs`.
    #[test]
    fn freeze_pins_identity_scheme_through_the_query_api() {
        let compiler = compiler_with_module_scope_types();
        let freeze = SemanticFreeze::freeze(&compiler).expect("resolved state freezes");

        for name in ["Point", "Alpha", "Color", "UserId", "First", "int", "Array"] {
            assert!(freeze.identity_of(name).is_some(), "{name} must be frozen");
        }
        // Identity is the canonical-descriptor hash, not an interning-order
        // ordinal: nominal identity is reproducible from the descriptor.
        assert_eq!(
            freeze.identity_of("Point"),
            Some(FrozenTypeIdentity::from_canonical_descriptor(
                "nominal:Point"
            ))
        );
        // Primitive synonyms coalesce; alias chains reach the terminal
        // identity; distinct types stay distinct.
        assert_eq!(freeze.identity_of("int"), freeze.identity_of("i64"));
        assert_eq!(freeze.identity_of("UserId"), freeze.identity_of("int"));
        assert_eq!(freeze.identity_of("First"), freeze.identity_of("int"));
        assert_ne!(freeze.identity_of("int"), freeze.identity_of("bool"));
        let color = freeze.identity_of("Color").expect("Color identity");
        assert_eq!(freeze.category_of(color), Ok(FrozenTypeCategory::Nominal));
        let never = freeze.identity_of("never").expect("never identity");
        assert_eq!(freeze.category_of(never), Ok(FrozenTypeCategory::Never));
    }

    /// S2: `comptime_freeze_overlay` performs the site-side discovery the
    /// deleted builder did — the enclosing generic function's declared type
    /// parameters enter as a scoped overlay (`parameter:{owner}:{name}`)
    /// over the shared base, without a rebuild.
    #[test]
    fn comptime_freeze_overlay_discovers_current_function_type_parameters() {
        let program = shape_ast::parse_program("fn identity<T>(value: T) -> T { value }")
            .expect("generic function parses");
        let shape_ast::ast::Item::Function(definition, _) = &program.items[0] else {
            panic!("expected function item");
        };
        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(definition)
            .expect("generic signature registers");
        compiler.current_function = compiler
            .program
            .functions
            .iter()
            .position(|function| function.name == "identity");
        compiler
            .install_semantic_freeze()
            .expect("registration-complete state freezes");

        let overlay = compiler
            .comptime_freeze_overlay()
            .expect("post-barrier site obtains the handle");
        let t = overlay.identity_of("T").expect("active T is discovered");
        assert_eq!(
            t,
            FrozenTypeIdentity::from_canonical_descriptor("parameter:identity:T"),
            "parameter identity must be owner-scoped"
        );
        assert_eq!(overlay.category_of(t), Ok(FrozenTypeCategory::Parameter));
        assert!(overlay.is_scoped_parameter("T"));
        // No rebuild: the overlay shares the installed freeze.
        assert!(Arc::ptr_eq(
            overlay.base(),
            compiler.semantic_freeze.as_ref().expect("freeze installed")
        ));
    }

    /// S2 rejection-matrix row 3: a comptime site reached without an
    /// installed freeze is a compile error with the named diagnostic —
    /// never an empty snapshot, never a per-site rebuild.
    #[test]
    fn comptime_site_without_freeze_handle_is_a_named_compile_error() {
        let compiler = BytecodeCompiler::new();
        let error = compiler
            .comptime_freeze_overlay()
            .expect_err("pre-barrier site must not obtain a handle");
        assert!(
            matches!(
                &error,
                ShapeError::TypeError(message)
                    if message.contains("no semantic freeze handle")
            ),
            "row-3 diagnostic missing: {error:?}"
        );
    }

    /// Overlay identities are scoped by owner (`parameter:{owner}:{name}`):
    /// same owner ⇒ same identity, different owner ⇒ different identity.
    /// Mirrors `parameter_identity_is_scoped_by_owning_function`.
    #[test]
    fn overlay_scopes_parameter_identities_by_owner() {
        let freeze = SemanticFreeze::freeze(&BytecodeCompiler::new()).expect("empty unit freezes");
        let params = vec!["T".to_string()];

        let map_a = FreezeOverlay::new(Arc::clone(&freeze), "map", &params);
        let map_b = FreezeOverlay::new(Arc::clone(&freeze), "map", &params);
        let filter = FreezeOverlay::new(Arc::clone(&freeze), "filter", &params);

        let map_t = map_a.identity_of("T").expect("map T identity");
        assert_eq!(map_b.identity_of("T"), Some(map_t));
        assert_ne!(filter.identity_of("T"), Some(map_t));
        assert_eq!(map_a.category_of(map_t), Ok(FrozenTypeCategory::Parameter));
    }

    /// The overlay shares the base index (no rebuild): base lookups pass
    /// through unchanged and a name already frozen in the base wins over a
    /// same-named parameter, matching the builder's interning order.
    #[test]
    fn overlay_reads_base_without_rebuilding() {
        let mut compiler = compiler_with_module_scope_types();
        compiler
            .struct_types
            .insert("T".to_string(), (Vec::new(), shape_ast::ast::Span::DUMMY));
        let freeze = SemanticFreeze::freeze(&compiler).expect("resolved state freezes");

        let overlay = FreezeOverlay::new(
            Arc::clone(&freeze),
            "map",
            &["T".to_string(), "U".to_string()],
        );

        // Same Arc — the base index was not rebuilt.
        assert!(Arc::ptr_eq(overlay.base(), &freeze));
        // Base names resolve identically through the overlay.
        assert_eq!(overlay.identity_of("int"), freeze.identity_of("int"));
        assert_eq!(overlay.identity_of("Point"), freeze.identity_of("Point"));
        // Base nominal `T` shadows the parameter `T` (interning-order parity).
        let t = overlay.identity_of("T").expect("T identity");
        assert_eq!(freeze.identity_of("T"), Some(t));
        assert_eq!(overlay.category_of(t), Ok(FrozenTypeCategory::Nominal));
        // `U` exists only in the overlay.
        let u = overlay.identity_of("U").expect("U identity");
        assert_eq!(freeze.identity_of("U"), None);
        assert_eq!(overlay.category_of(u), Ok(FrozenTypeCategory::Parameter));
    }

    /// ADR-009 B1 S2: `payload_of` grows the ONE query API beside
    /// `identity_of`/`category_of` — base half on `SemanticFreeze`, overlay
    /// half on `FreezeOverlay`. Enabled categories return complete typed
    /// payloads; ADR-009 B7 Slice 2 makes a scoped Parameter identity answer a
    /// complete `TypeParamDescriptor` off its stable base-fn-scoped identity
    /// (never an inference hole, provably-empty bounds); nominal defers to the
    /// base. Never a partial descriptor.
    #[test]
    fn payload_query_grows_the_shared_query_api() {
        use super::super::type_reflection::payloads::{
            FrozenPayloadDescriptor, TypeParamDescriptor,
        };
        use shape_runtime::comptime_reflection::{FrozenPrimitive, IntegerWidth};

        let compiler = compiler_with_module_scope_types();
        let freeze = SemanticFreeze::freeze(&compiler).expect("resolved state freezes");
        let overlay = FreezeOverlay::new(Arc::clone(&freeze), "map", &["T".to_string()]);

        // Base half and overlay half agree for base identities.
        let int_identity = freeze.identity_of("int").expect("int identity");
        assert_eq!(
            freeze.payload_of(int_identity),
            Ok(FrozenPayloadDescriptor::Primitive(
                FrozenPrimitive::SignedInteger(IntegerWidth::W64)
            ))
        );
        assert_eq!(
            overlay.payload_of(int_identity),
            freeze.payload_of(int_identity)
        );

        // Overlay half (B7 Slice 2): a scoped Parameter identity answers a
        // complete `TypeParamDescriptor` — the queried identity itself, with a
        // provably-empty bound set (never a partial descriptor).
        let t = overlay.identity_of("T").expect("T identity");
        assert_eq!(
            overlay.payload_of(t),
            Ok(FrozenPayloadDescriptor::Parameter(TypeParamDescriptor {
                identity: t,
                bounds: Vec::new(),
            })),
            "a scoped Parameter identity must answer its own stable identity"
        );

        // Base half: ADR-009 B5 — a base user nominal answers a positive
        // sealed shape descriptor (a multi-field struct is `Struct`).
        let point = freeze.identity_of("Point").expect("Point identity");
        match freeze.payload_of(point).expect("Nominal must answer a shape") {
            FrozenPayloadDescriptor::Nominal(
                super::super::type_reflection::payloads::NominalDescriptor::Struct { owner, .. },
            ) => assert_eq!(owner, point),
            other => panic!("multi-field Point must be Struct, got {other:?}"),
        }
    }

    /// Rejection-matrix row 4 (Dec 52): freezing partial semantic state is a
    /// named-diagnostic error, never a populated freeze.
    #[test]
    fn unresolved_inference_variable_cannot_be_frozen() {
        let mut compiler = BytecodeCompiler::new();
        let unresolved = TypeVarGen::new().fresh_var();
        compiler.struct_types.insert(
            "Aabb".to_string(),
            (vec!["min".to_string()], shape_ast::ast::Span::DUMMY),
        );
        compiler.struct_generic_info.insert(
            "Aabb".to_string(),
            crate::compiler::StructGenericInfo {
                type_params: Vec::new(),
                runtime_field_types: [("min".to_string(), tyvar_to_annotation(&unresolved))]
                    .into_iter()
                    .collect(),
            },
        );

        let error =
            SemanticFreeze::freeze(&compiler).expect_err("partial semantic state must not freeze");
        let diagnostic = error.diagnostic();
        assert!(
            diagnostic.contains("cannot be frozen"),
            "named Dec 52 class missing from: {diagnostic}"
        );
        assert!(
            diagnostic.contains("unresolved inference variable"),
            "named Dec 52 cause missing from: {diagnostic}"
        );
        assert!(
            diagnostic.contains("Aabb") && diagnostic.contains("min"),
            "diagnostic must name the frozen subject: {diagnostic}"
        );
    }

    /// Row 4 recursion: the marker is found structurally, not only at the
    /// top level of an annotation.
    #[test]
    fn nested_unresolved_inference_variable_cannot_be_frozen() {
        let mut compiler = BytecodeCompiler::new();
        let unresolved = TypeVarGen::new().fresh_var();
        compiler.struct_types.insert(
            "Holder".to_string(),
            (vec!["items".to_string()], shape_ast::ast::Span::DUMMY),
        );
        compiler.struct_generic_info.insert(
            "Holder".to_string(),
            crate::compiler::StructGenericInfo {
                type_params: Vec::new(),
                runtime_field_types: [(
                    "items".to_string(),
                    TypeAnnotation::Array(Box::new(tyvar_to_annotation(&unresolved))),
                )]
                .into_iter()
                .collect(),
            },
        );

        let error = SemanticFreeze::freeze(&compiler)
            .expect_err("nested partial semantic state must not freeze");
        assert!(error.diagnostic().contains("unresolved inference variable"));
    }

    #[test]
    fn tampered_reserved_type_variable_carrier_fails_closed_at_freeze() {
        let mut compiler = BytecodeCompiler::new();
        let unresolved = TypeVarGen::new().fresh_var();
        let TypeAnnotation::Basic(mut tampered) = tyvar_to_annotation(&unresolved) else {
            panic!("type-variable carrier must use the Basic annotation form")
        };
        tampered.push('0');
        let tampered_leaf = TypeAnnotation::Basic(tampered);
        assert!(
            annotation_as_tyvar(&tampered_leaf).is_none(),
            "tampered carrier must not recover TypeVar authority"
        );
        let tampered = TypeAnnotation::Array(Box::new(tampered_leaf));
        compiler.struct_types.insert(
            "TamperedHolder".to_string(),
            (vec!["item".to_string()], shape_ast::ast::Span::DUMMY),
        );
        compiler.struct_generic_info.insert(
            "TamperedHolder".to_string(),
            crate::compiler::StructGenericInfo {
                type_params: Vec::new(),
                runtime_field_types: [("item".to_string(), tampered)].into_iter().collect(),
            },
        );

        let error = SemanticFreeze::freeze(&compiler)
            .expect_err("reserved carrier with an invalid MAC must fail closed");
        let diagnostic = error.diagnostic();
        assert!(diagnostic.contains("unresolved inference variable"));
        assert!(diagnostic.contains("TamperedHolder") && diagnostic.contains("item"));
    }

    // ========================================================================
    // ADR-009 A2 (slice S2): FreezeOverlay composite-category query-API
    // extension. Composite identities minted by `canonicalize_type` are
    // answered by the SAME `category_of` query (resolution order: scoped
    // parameters → site-interned composites → base) — one query API per
    // spec §4.1, no new lookup entry point, no per-site rebuild, no
    // Default/empty construction path (rejection-matrix row 9).
    // ========================================================================

    /// (a) Canonicalize-and-intern of a composite through the overlay makes
    /// `category_of(identity)` answer the structural category, while scoped
    /// parameters and base leaves keep answering exactly as before. The
    /// shared base freeze is never mutated (site-scoped memo, no rebuild).
    #[test]
    fn canonicalized_composite_identity_is_answered_by_the_same_category_query() {
        use shape_ast::ast::TypePath;
        let compiler = compiler_with_module_scope_types();
        let freeze = SemanticFreeze::freeze(&compiler).expect("resolved state freezes");
        let overlay = FreezeOverlay::new(Arc::clone(&freeze), "map", &["T".to_string()]);

        let forms: Vec<(TypeAnnotation, FrozenTypeCategory)> = vec![
            (
                TypeAnnotation::Tuple(vec![
                    TypeAnnotation::Basic("int".to_string()),
                    TypeAnnotation::Basic("string".to_string()),
                ]),
                FrozenTypeCategory::Tuple,
            ),
            (
                TypeAnnotation::Union(vec![
                    TypeAnnotation::Basic("int".to_string()),
                    TypeAnnotation::Basic("string".to_string()),
                ]),
                FrozenTypeCategory::Union,
            ),
            (
                TypeAnnotation::Borrow {
                    mutable: false,
                    inner: Box::new(TypeAnnotation::Basic("Point".to_string())),
                },
                FrozenTypeCategory::Reference,
            ),
            (
                TypeAnnotation::Generic {
                    name: TypePath::simple("Option"),
                    args: vec![TypeAnnotation::Basic("int".to_string())],
                },
                FrozenTypeCategory::Nominal,
            ),
        ];
        for (annotation, expected) in forms {
            let identity = overlay
                .canonicalize_type(&annotation)
                .expect("composite type expression canonicalizes");
            assert_eq!(
                overlay.category_of(identity),
                Ok(expected),
                "the ONE category query must answer the interned composite"
            );
            // Site-scoped: the shared base freeze is NOT mutated by interning.
            assert!(
                freeze.category_of(identity).is_err(),
                "base freeze must not learn site-interned composite identities"
            );
        }
        // Scoped parameters and base leaves resolve through the same query,
        // unchanged by a populated composite memo.
        let t = overlay.identity_of("T").expect("scoped parameter T");
        assert_eq!(overlay.category_of(t), Ok(FrozenTypeCategory::Parameter));
        let point = overlay.identity_of("Point").expect("base nominal Point");
        assert_eq!(overlay.category_of(point), Ok(FrozenTypeCategory::Nominal));
    }

    /// (b) An identity never interned anywhere still rejects through the
    /// SAME query with the EXACT named diagnostic text — pinned by
    /// `frozen_type.rs` row-2 and
    /// `lsp::typed_comptime::unresolved_type_ref_has_semantic_diagnostic`.
    /// A populated memo must not change the rejection for identities it
    /// does not hold.
    #[test]
    fn un_interned_identity_keeps_the_named_unknown_identity_rejection() {
        let compiler = compiler_with_module_scope_types();
        let freeze = SemanticFreeze::freeze(&compiler).expect("resolved state freezes");
        let overlay = FreezeOverlay::new(Arc::clone(&freeze), "<module>", &[]);
        overlay
            .canonicalize_type(&TypeAnnotation::Tuple(vec![
                TypeAnnotation::Basic("int".to_string()),
                TypeAnnotation::Basic("string".to_string()),
            ]))
            .expect("composite type expression canonicalizes");

        let named = "type_ref received an unknown semantic type identity".to_string();
        let never_interned =
            FrozenTypeIdentity::from_canonical_descriptor("test:never-interned-identity");
        assert_eq!(overlay.category_of(never_interned), Err(named.clone()));
        assert_eq!(overlay.category_of(FrozenTypeIdentity::INVALID), Err(named));
    }

    /// (c) A3 declaration-stability: the same applied-over-Parameter form
    /// interned through two overlays with the SAME owner yields the
    /// identical identity; a different owner yields a different one.
    #[test]
    fn applied_over_parameter_identity_is_stable_across_same_owner_overlays() {
        use shape_ast::ast::TypePath;
        let freeze = SemanticFreeze::freeze(&BytecodeCompiler::new()).expect("empty unit freezes");
        let params = vec!["T".to_string()];
        let annotation = TypeAnnotation::Generic {
            name: TypePath::simple("Option"),
            args: vec![TypeAnnotation::Basic("T".to_string())],
        };

        let map_a = FreezeOverlay::new(Arc::clone(&freeze), "map", &params);
        let map_b = FreezeOverlay::new(Arc::clone(&freeze), "map", &params);
        let filter = FreezeOverlay::new(Arc::clone(&freeze), "filter", &params);

        let a = map_a
            .canonicalize_type(&annotation)
            .expect("applied-over-parameter form canonicalizes");
        let b = map_b
            .canonicalize_type(&annotation)
            .expect("applied-over-parameter form canonicalizes");
        assert_eq!(
            a, b,
            "same owner must yield the identical applied-over-parameter identity"
        );
        assert_eq!(map_a.category_of(a), Ok(FrozenTypeCategory::Nominal));
        assert_eq!(map_b.category_of(b), Ok(FrozenTypeCategory::Nominal));

        let f = filter
            .canonicalize_type(&annotation)
            .expect("applied-over-parameter form canonicalizes");
        assert_ne!(
            f, a,
            "a different owner embeds a different parameter identity"
        );
    }

    /// (d) Rejection-matrix row 9: the S2 memo adds NO Default/empty
    /// construction path — no freeze surface implements `Default`.
    /// Method-resolution detector: the inherent `implements_default` exists
    /// only when `T: Default`; otherwise the trait method answers false.
    #[test]
    fn s2_memo_adds_no_default_or_empty_construction_path() {
        struct DefaultDetector<T>(std::marker::PhantomData<T>);
        trait NoDefaultConstruction {
            fn implements_default(&self) -> bool {
                false
            }
        }
        impl<T> NoDefaultConstruction for DefaultDetector<T> {}
        impl<T: Default> DefaultDetector<T> {
            fn implements_default(&self) -> bool {
                true
            }
        }

        assert!(
            !DefaultDetector::<FreezeOverlay>(std::marker::PhantomData).implements_default(),
            "FreezeOverlay must not grow a Default/empty construction path"
        );
        assert!(
            !DefaultDetector::<SemanticFreeze>(std::marker::PhantomData).implements_default(),
            "SemanticFreeze must not grow a Default/empty construction path"
        );
        assert!(
            !DefaultDetector::<FrozenTypeIndex>(std::marker::PhantomData).implements_default(),
            "FrozenTypeIndex must not grow a Default/empty construction path"
        );
        // Sanity: the detector does report a real Default impl.
        assert!(
            DefaultDetector::<HashMap<String, FrozenTypeIdentity>>(std::marker::PhantomData)
                .implements_default()
        );
    }

    /// The barrier installs the freeze exactly once per compilation unit; a
    /// second install is an internal-invariant compile error.
    #[test]
    fn semantic_freeze_installs_exactly_once_at_registration_barrier() {
        let mut compiler = BytecodeCompiler::new();
        compiler
            .install_semantic_freeze()
            .expect("first install succeeds");
        assert!(
            compiler.semantic_freeze.is_some(),
            "freeze handle installed"
        );

        let error = compiler
            .install_semantic_freeze()
            .expect_err("second install is rejected");
        assert!(
            matches!(&error, ShapeError::TypeError(message) if message.contains("exactly once")),
            "unexpected error: {error:?}"
        );
    }

    // ── ADR-009 (ticket B2, slice S1): barrier-time trait/impl truth. ──
    //
    // Trait definitions and trait-impl registrations must be visible in
    // `compiler.type_inference.env` AT `install_semantic_freeze` time — for
    // both entry points. Before S1, trait defs registered in
    // `register_item_functions` (after the barrier) and impls later still
    // (pass-2 `Item::Impl`), so S2's trait-identity / impl-evidence freeze
    // inputs would have frozen an EMPTY table — a Dec 52 ordering violation
    // that would masquerade as `find_impl` → None.

    /// Root (`compile()`) entry point: trait + default impl + named impl
    /// truth is in the env when the barrier runs.
    #[test]
    fn trait_and_impl_truth_is_in_env_at_barrier_for_root_program() {
        let source = r#"
type User { name: string }
trait Greetable {
    method greet() -> string;
}
impl Greetable for User {
    method greet() { "Hello, " + self.name }
}
impl Greetable for User as Loud {
    method greet() { "HELLO, " + self.name }
}
let u = User { name: "Alice" }
u.greet()
"#;
        let program = shape_ast::parse_program(source).expect("program parses");
        let compiler = BytecodeCompiler::new();
        compiler.compile(&program).expect("program compiles");

        let truth = barrier_env_truth_for_tests::take()
            .expect("compile() must run the semantic-freeze barrier");
        assert!(
            truth.trait_names.contains("Greetable"),
            "trait def must be registered in env at the barrier; saw: {:?}",
            truth.trait_names
        );
        assert!(
            truth.trait_impl_keys.contains("Greetable::User"),
            "default impl must be registered in env at the barrier; saw: {:?}",
            truth.trait_impl_keys
        );
        assert!(
            truth.trait_impl_keys.contains("Greetable::User::Loud"),
            "named impl must be registered (canonical 3-part key) at the barrier; saw: {:?}",
            truth.trait_impl_keys
        );
    }

    /// Two-sub-pass ordering: an impl declared BEFORE its trait in source
    /// order still registers at the barrier (all traits predeclare first,
    /// then impls, so `register_trait_impl` validation never fires against
    /// an unregistered trait).
    #[test]
    fn impl_declared_before_its_trait_is_barrier_truth() {
        let source = r#"
type User { name: string }
impl Greetable for User {
    method greet() { "Hello, " + self.name }
}
trait Greetable {
    method greet() -> string;
}
let u = User { name: "Alice" }
u.greet()
"#;
        let program = shape_ast::parse_program(source).expect("program parses");
        let compiler = BytecodeCompiler::new();
        // Compile outcome is not the subject here (source order of trait vs
        // impl may be diagnosed elsewhere); the barrier must still have run
        // over trait-complete state.
        let _ = compiler.compile(&program);

        let truth = barrier_env_truth_for_tests::take()
            .expect("compile() must run the semantic-freeze barrier");
        assert!(
            truth.trait_names.contains("Greetable"),
            "trait declared after the impl must still be barrier truth; saw: {:?}",
            truth.trait_names
        );
        assert!(
            truth.trait_impl_keys.contains("Greetable::User"),
            "impl declared before its trait must still be barrier truth; saw: {:?}",
            truth.trait_impl_keys
        );
    }

    /// Two-sub-pass validation: an INVALID impl (missing a required method)
    /// is NOT barrier truth even when it precedes its trait in source order
    /// — because traits predeclare first, `register_trait_impl` validation
    /// sees the trait def and rejects the impl.
    #[test]
    fn invalid_impl_before_its_trait_is_not_barrier_truth() {
        let source = r#"
type User { name: string }
impl Greetable for User {
}
trait Greetable {
    method greet() -> string;
}
let x = 1
x
"#;
        let program = shape_ast::parse_program(source).expect("program parses");
        let compiler = BytecodeCompiler::new();
        // The program is invalid (impl misses `greet`); the analyzer reports
        // it AFTER the barrier. Only the barrier-time truth is asserted.
        let _ = compiler.compile(&program);

        let truth = barrier_env_truth_for_tests::take()
            .expect("compile() must run the semantic-freeze barrier");
        assert!(
            truth.trait_names.contains("Greetable"),
            "trait must be barrier truth; saw: {:?}",
            truth.trait_names
        );
        assert!(
            !truth.trait_impl_keys.contains("Greetable::User"),
            "an impl missing a required method must NOT be barrier truth; saw: {:?}",
            truth.trait_impl_keys
        );
    }

    // ── ADR-009 (ticket B2, slice S2): freeze inputs 4/5 — distinct trait
    // identities + impl evidence. ──

    fn trait_def(name: &str) -> shape_ast::ast::TraitDef {
        shape_ast::ast::TraitDef {
            name: name.to_string(),
            doc_comment: None,
            type_params: None,
            super_traits: Vec::new(),
            members: Vec::new(),
            annotations: Vec::new(),
            is_comptime: false,
        }
    }

    fn add_struct(compiler: &mut BytecodeCompiler, name: &str) {
        compiler
            .struct_types
            .insert(name.to_string(), (Vec::new(), shape_ast::ast::Span::DUMMY));
    }

    /// Freeze input 4 (Dec 49 / Dec 50 rule 5): trait identities are a
    /// DISTINCT identity kind — stable across builds (canonical
    /// `trait:{name}` descriptor hash, not an ordinal), distinct from a
    /// same-named struct's type identity, NEVER interned into
    /// `frozen_type_ids` (`type_ref(TraitName)` keeps failing), and with NO
    /// `FrozenTypeCategory` (there is no `Trait` variant).
    #[test]
    fn trait_identity_is_a_distinct_stable_identity_kind() {
        let build = || {
            let mut compiler = BytecodeCompiler::new();
            // Same-named VALUE type next to the trait.
            add_struct(&mut compiler, "Greetable");
            compiler
                .type_inference
                .env
                .define_trait(&trait_def("Greetable"));
            compiler
                .type_inference
                .env
                .define_trait(&trait_def("Serializable"));
            SemanticFreeze::freeze(&compiler).expect("resolved state freezes")
        };
        let freeze = build();

        let greetable_trait = freeze
            .trait_identity_of("Greetable")
            .expect("trait identity frozen");
        // Stable across builds: reproducible from the canonical descriptor.
        assert_eq!(
            build().trait_identity_of("Greetable"),
            Some(greetable_trait)
        );
        assert_eq!(
            greetable_trait,
            FrozenTypeIdentity::from_canonical_descriptor("trait:Greetable")
        );
        // Distinct from the same-named struct's type identity.
        let greetable_type = freeze.identity_of("Greetable").expect("nominal identity");
        assert_ne!(greetable_trait, greetable_type);
        // Distinct kind: a trait-only name never enters `frozen_type_ids`…
        assert!(freeze.trait_identity_of("Serializable").is_some());
        assert_eq!(freeze.identity_of("Serializable"), None);
        // …and a trait identity has no `FrozenTypeCategory` (Dec 50 rule 5).
        assert!(freeze.category_of(greetable_trait).is_err());
        // Types are not traits either: value-type names have no trait identity.
        assert_eq!(freeze.trait_identity_of("int"), None);
    }

    /// Freeze input 5: named impls (`impl Trait for Type as Name`) freeze as
    /// DISTINCT evidence identities next to the default impl, all through the
    /// canonical `impl:{trait}:{type}:{impl_name_or_default}` descriptor.
    #[test]
    fn named_impls_freeze_distinct_evidence_identities() {
        let mut compiler = BytecodeCompiler::new();
        add_struct(&mut compiler, "User");
        compiler
            .type_inference
            .env
            .define_trait(&trait_def("Greetable"));
        compiler
            .type_inference
            .env
            .register_trait_impl("Greetable", "User", vec!["greet".to_string()])
            .expect("default impl registers");
        compiler
            .type_inference
            .env
            .register_trait_impl_named("Greetable", "User", "Loud", vec!["greet".to_string()])
            .expect("named impl registers");
        let freeze = SemanticFreeze::freeze(&compiler).expect("resolved state freezes");

        let trait_id = freeze.trait_identity_of("Greetable").expect("trait id");
        let type_id = freeze.identity_of("User").expect("type id");
        let set = freeze
            .impl_evidence_of(trait_id, type_id)
            .expect("implemented pair is never a surface-and-stop")
            .expect("implemented pair has evidence");
        let default_impl = set.default_impl().expect("default impl evidence");
        assert_eq!(
            default_impl.identity,
            FrozenTypeIdentity::from_canonical_descriptor("impl:Greetable:User:__default__")
        );
        assert_eq!(default_impl.method_names, vec!["greet".to_string()]);
        assert_eq!(set.named_impls().len(), 1);
        let named = &set.named_impls()[0];
        assert_eq!(named.impl_name.as_deref(), Some("Loud"));
        assert_eq!(
            named.identity,
            FrozenTypeIdentity::from_canonical_descriptor("impl:Greetable:User:Loud")
        );
        assert_ne!(named.identity, default_impl.identity);
        // Impl identities are their own kind: never the trait's or type's.
        for identity in [default_impl.identity, named.identity] {
            assert_ne!(identity, trait_id);
            assert_ne!(identity, type_id);
        }
    }

    /// Freeze input 5, negative half: an unimplemented pair has NO evidence
    /// entry — `Ok(None)`, never a fabricated/partial entry and never an
    /// error for a genuine miss.
    #[test]
    fn unimplemented_pair_has_no_evidence_entry() {
        let mut compiler = BytecodeCompiler::new();
        add_struct(&mut compiler, "User");
        add_struct(&mut compiler, "Order");
        compiler
            .type_inference
            .env
            .define_trait(&trait_def("Greetable"));
        compiler
            .type_inference
            .env
            .register_trait_impl("Greetable", "User", Vec::new())
            .expect("impl registers");
        let freeze = SemanticFreeze::freeze(&compiler).expect("resolved state freezes");

        let trait_id = freeze.trait_identity_of("Greetable").expect("trait id");
        let user = freeze.identity_of("User").expect("User id");
        let order = freeze.identity_of("Order").expect("Order id");
        assert!(
            freeze
                .impl_evidence_of(trait_id, user)
                .expect("implemented pair resolves")
                .is_some()
        );
        assert!(
            freeze
                .impl_evidence_of(trait_id, order)
                .expect("genuine miss is not an error")
                .is_none()
        );
        assert!(
            freeze
                .impl_evidence_of(trait_id, freeze.identity_of("string").expect("string id"))
                .expect("genuine miss is not an error")
                .is_none()
        );
    }

    /// Ruled stance (B2 S2): blanket-impl satisfaction does NOT silently
    /// become implementation evidence — a pair that only a blanket impl
    /// could satisfy is a NAMED surface-and-stop diagnostic, never a silent
    /// `None` (considered-compromise log lands in defections.md, S6).
    #[test]
    fn blanket_impl_satisfaction_is_not_frozen_evidence() {
        let mut compiler = BytecodeCompiler::new();
        add_struct(&mut compiler, "User");
        add_struct(&mut compiler, "Order");
        compiler
            .type_inference
            .env
            .define_trait(&trait_def("Printable"));
        compiler
            .type_inference
            .env
            .register_blanket_impl("Printable", Vec::new(), Vec::new());
        compiler
            .type_inference
            .env
            .register_trait_impl("Printable", "Order", Vec::new())
            .expect("direct impl registers");
        let freeze = SemanticFreeze::freeze(&compiler).expect("resolved state freezes");

        let trait_id = freeze.trait_identity_of("Printable").expect("trait id");
        // Direct evidence still answers for the directly-implemented pair.
        assert!(
            freeze
                .impl_evidence_of(trait_id, freeze.identity_of("Order").expect("Order id"))
                .expect("direct impl resolves")
                .is_some()
        );
        // Blanket-only pair: named diagnostic.
        let error = freeze
            .impl_evidence_of(trait_id, freeze.identity_of("User").expect("User id"))
            .expect_err("blanket-only satisfaction must surface-and-stop");
        assert!(
            error.contains("blanket-impl satisfaction is not frozen implementation evidence"),
            "named blanket stance missing from: {error}"
        );
    }

    /// Ruled stance (B2 S2): the legacy `implements` int→number widening
    /// rule does NOT silently become evidence — a NAMED surface-and-stop
    /// diagnostic, never a silent `None` (E5 deletes the legacy rule).
    #[test]
    fn legacy_numeric_widening_is_not_frozen_evidence() {
        let mut compiler = BytecodeCompiler::new();
        compiler
            .type_inference
            .env
            .define_trait(&trait_def("Scalable"));
        compiler
            .type_inference
            .env
            .register_trait_impl("Scalable", "number", Vec::new())
            .expect("impl registers");
        let freeze = SemanticFreeze::freeze(&compiler).expect("resolved state freezes");

        let trait_id = freeze.trait_identity_of("Scalable").expect("trait id");
        assert!(
            freeze
                .impl_evidence_of(trait_id, freeze.identity_of("number").expect("number id"))
                .expect("direct impl resolves")
                .is_some()
        );
        let error = freeze
            .impl_evidence_of(trait_id, freeze.identity_of("int").expect("int id"))
            .expect_err("widening satisfaction must surface-and-stop");
        assert!(
            error.contains("legacy numeric widening is not frozen implementation evidence"),
            "named widening stance missing from: {error}"
        );
        // A non-numeric miss stays a plain genuine None.
        assert!(
            freeze
                .impl_evidence_of(trait_id, freeze.identity_of("string").expect("string id"))
                .expect("genuine miss is not an error")
                .is_none()
        );
    }

    /// Impl entries register their trait name AS WRITTEN (unqualified) while
    /// dep-module trait defs register qualified (`qualify_module_item`
    /// qualifies the impl TARGET but not the trait name — B2 S1). Evidence
    /// resolution anchors the impl to the qualified frozen trait:
    /// module-relative first (the impl lives in its target's module), then
    /// unique suffix.
    #[test]
    fn dep_module_impl_anchors_to_the_qualified_frozen_trait() {
        let mut compiler = BytecodeCompiler::new();
        add_struct(&mut compiler, "calc::numbers::User");
        compiler
            .type_inference
            .env
            .define_trait(&trait_def("calc::numbers::Greetable"));
        compiler
            .type_inference
            .env
            .register_trait_impl("Greetable", "calc::numbers::User", Vec::new())
            .expect("impl registers");
        let freeze = SemanticFreeze::freeze(&compiler).expect("resolved state freezes");

        let trait_id = freeze
            .trait_identity_of("calc::numbers::Greetable")
            .expect("qualified trait id");
        let type_id = freeze
            .identity_of("calc::numbers::User")
            .expect("qualified type id");
        let set = freeze
            .impl_evidence_of(trait_id, type_id)
            .expect("resolves")
            .expect("evidence frozen under the qualified pair");
        let evidence = set.default_impl().expect("default impl evidence");
        assert_eq!(evidence.trait_name, "calc::numbers::Greetable");
        assert_eq!(
            evidence.identity,
            FrozenTypeIdentity::from_canonical_descriptor(
                "impl:calc::numbers::Greetable:calc::numbers::User:__default__"
            )
        );
    }

    /// An impl whose as-written trait name matches MORE THAN ONE frozen
    /// trait def cannot be attributed: the affected candidate pairs are
    /// poisoned so the QUERY is a named surface-and-stop — never a guess,
    /// never a silent miss.
    #[test]
    fn ambiguous_unqualified_impl_trait_is_query_time_surface_and_stop() {
        let mut compiler = BytecodeCompiler::new();
        add_struct(&mut compiler, "c::User");
        compiler
            .type_inference
            .env
            .define_trait(&trait_def("a::Marker"));
        compiler
            .type_inference
            .env
            .define_trait(&trait_def("b::Marker"));
        compiler
            .type_inference
            .env
            .register_trait_impl("Marker", "c::User", Vec::new())
            .expect("impl registers");
        let freeze = SemanticFreeze::freeze(&compiler).expect("resolved state freezes");

        let type_id = freeze.identity_of("c::User").expect("type id");
        for name in ["a::Marker", "b::Marker"] {
            let trait_id = freeze.trait_identity_of(name).expect("candidate trait id");
            let error = freeze
                .impl_evidence_of(trait_id, type_id)
                .expect_err("ambiguous attribution must not guess or silently miss");
            assert!(
                error.contains("ambiguous trait-impl evidence"),
                "named ambiguity diagnostic missing from: {error}"
            );
        }
    }

    /// Synonym-target registrations (`number`/`f64`) coalesce to ONE frozen
    /// evidence slot when their facts are identical…
    #[test]
    fn synonym_target_impls_with_identical_facts_coalesce() {
        let mut compiler = BytecodeCompiler::new();
        compiler
            .type_inference
            .env
            .define_trait(&trait_def("Scalable"));
        compiler
            .type_inference
            .env
            .register_trait_impl("Scalable", "number", vec!["abs".to_string()])
            .expect("impl registers");
        compiler
            .type_inference
            .env
            .register_trait_impl("Scalable", "f64", vec!["abs".to_string()])
            .expect("synonym impl registers");
        let freeze = SemanticFreeze::freeze(&compiler).expect("identical facts coalesce");

        let trait_id = freeze.trait_identity_of("Scalable").expect("trait id");
        let set = freeze
            .impl_evidence_of(trait_id, freeze.identity_of("number").expect("number id"))
            .expect("resolves")
            .expect("evidence frozen");
        assert!(set.default_impl().is_some());
        assert!(set.named_impls().is_empty());
    }

    /// …but registrations that collapse to the same canonical slot with
    /// DIFFERING facts reject the whole freeze (Dec 52: never an ambiguously
    /// populated freeze).
    #[test]
    fn conflicting_impl_facts_reject_the_freeze() {
        let mut compiler = BytecodeCompiler::new();
        compiler
            .type_inference
            .env
            .define_trait(&trait_def("Scalable"));
        compiler
            .type_inference
            .env
            .register_trait_impl("Scalable", "number", vec!["abs".to_string()])
            .expect("impl registers");
        compiler
            .type_inference
            .env
            .register_trait_impl("Scalable", "f64", vec!["ceil".to_string()])
            .expect("synonym impl registers (registry keys by string)");

        let error = SemanticFreeze::freeze(&compiler)
            .expect_err("conflicting canonical evidence must not freeze");
        assert!(
            error.diagnostic().contains("conflicting trait-impl facts"),
            "named conflict diagnostic missing from: {}",
            error.diagnostic()
        );
        assert!(
            error.diagnostic().contains("Scalable"),
            "diagnostic must name the subject: {}",
            error.diagnostic()
        );
    }

    /// Trait/evidence queries pass through the overlay unchanged (traits are
    /// a distinct identity kind — scoped generic parameters never shadow
    /// them) and the A3 specialization overlay still composes.
    #[test]
    fn overlay_passes_trait_queries_through_and_composes_with_specialization_overlay() {
        let mut compiler = BytecodeCompiler::new();
        add_struct(&mut compiler, "User");
        compiler
            .type_inference
            .env
            .define_trait(&trait_def("Greetable"));
        compiler
            .type_inference
            .env
            .register_trait_impl("Greetable", "User", Vec::new())
            .expect("impl registers");
        compiler
            .install_semantic_freeze()
            .expect("registration-complete state freezes");
        let _specialization = compiler.specialization_type_overlays.enter(
            SpecializationTypeOverlay::declaration_only("map", vec!["T".to_string()]),
        );

        let overlay = compiler
            .comptime_freeze_overlay()
            .expect("post-barrier site obtains the handle");
        // A3 composition: the specialization overlay still supplies T.
        let t = overlay.identity_of("T").expect("specialized T identity");
        assert_eq!(
            t,
            FrozenTypeIdentity::from_canonical_descriptor("parameter:map:T")
        );
        // Trait queries pass through to the shared base freeze.
        let trait_id = overlay
            .trait_identity_of("Greetable")
            .expect("trait id through overlay");
        assert_eq!(
            overlay.base().trait_identity_of("Greetable"),
            Some(trait_id)
        );
        let type_id = overlay.identity_of("User").expect("type id");
        assert!(
            overlay
                .impl_evidence_of(trait_id, type_id)
                .expect("evidence through overlay")
                .is_some()
        );
    }

    /// Ruled stance (B2 S1/S2): the freeze is FREEZE-TIME truth — an impl
    /// registered after the barrier (comptime-generated / annotation /
    /// extend families) is not frozen evidence. The freeze factually reports
    /// no evidence; slice S5 lands the named diagnostic that keeps this from
    /// masquerading as `find_impl` → None at the public surface.
    #[test]
    fn post_barrier_impl_registration_is_not_frozen_evidence() {
        let mut compiler = BytecodeCompiler::new();
        add_struct(&mut compiler, "User");
        compiler
            .type_inference
            .env
            .define_trait(&trait_def("Greetable"));
        compiler
            .install_semantic_freeze()
            .expect("registration-complete state freezes");
        // Post-barrier registration (the annotation/extend/comptime family).
        compiler
            .type_inference
            .env
            .register_trait_impl("Greetable", "User", Vec::new())
            .expect("post-barrier impl registers into the live env");

        let freeze = Arc::clone(compiler.semantic_freeze.as_ref().expect("freeze installed"));
        let trait_id = freeze.trait_identity_of("Greetable").expect("trait id");
        let type_id = freeze.identity_of("User").expect("type id");
        assert!(
            freeze
                .impl_evidence_of(trait_id, type_id)
                .expect("freeze-time truth resolves")
                .is_none(),
            "post-barrier registration must not appear as frozen evidence"
        );
    }

    /// Graph (`compile_with_graph_and_prelude`) entry point — the
    /// A1-review-round-1 regression class: trait/impl truth declared in an
    /// IMPORTED dependency module is in the env when the pre-Phase-1 unit
    /// barrier runs (dep traits register under their qualified names; the
    /// impl's trait name stays as written, per `qualify_module_item`).
    #[test]
    fn trait_and_impl_truth_is_in_env_at_barrier_for_imported_dep_module() {
        let numbers = r#"
pub type User { name: string }
pub trait Greetable {
    method greet() -> string;
}
impl Greetable for User {
    method greet() { "Hello, " + self.name }
}
pub fn seven() -> int { 7 }
"#;
        let main = r#"
from calc::numbers use { seven }
seven()
"#;
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("calc")).unwrap();
        std::fs::write(dir.path().join("calc/numbers.shape"), numbers).unwrap();

        let program = shape_ast::parse_program(main).expect("main parses");
        let mut loader = shape_runtime::module_loader::ModuleLoader::new();
        loader.add_module_path(dir.path().to_path_buf());
        let (graph, stdlib_names, prelude_imports) =
            crate::module_resolution::build_graph_and_stdlib_names(&program, &mut loader, &[])
                .expect("graph builds");
        let mut compiler = BytecodeCompiler::new();
        compiler.stdlib_function_names = stdlib_names;
        compiler
            .compile_with_graph_and_prelude(&program, graph, &prelude_imports)
            .expect("graph program compiles");

        let truth = barrier_env_truth_for_tests::take()
            .expect("graph entry point must run the semantic-freeze barrier");
        assert!(
            truth.trait_names.contains("calc::numbers::Greetable"),
            "dep-module trait must be barrier truth under its qualified name; saw: {:?}",
            truth.trait_names
        );
        assert!(
            truth
                .trait_impl_keys
                .contains("Greetable::calc::numbers::User"),
            "dep-module impl must be barrier truth (trait as written, target qualified); saw: {:?}",
            truth.trait_impl_keys
        );
    }

    /// B2 S6: every B2 stance/rejection diagnostic raised on the comptime
    /// mini-VM execution path must survive the comptime diagnostics firewall
    /// unchanged. The firewall (`helpers.rs::sanitize_comptime_internal`,
    /// reached via `clean_comptime_message` on every failed comptime
    /// execution) wholesale-replaces any message containing internal jargon
    /// markers ("ADR-", "§", …) with a generic not-available sentence — a
    /// named surface-and-stop whose text gets masked is a silent diagnostic
    /// regression at the user surface.
    ///
    /// Deliberately NOT in the list: `NO_FREEZE_HANDLE_DIAGNOSTIC` (A1). It
    /// is raised as a `CompileError` from `comptime_freeze_overlay` BEFORE
    /// any mini-VM execution, so it never routes through
    /// `clean_comptime_message`; its "(ADR-009 §4.1)" citation is an A1
    /// compile-error surface outside this ticket's territory.
    #[test]
    fn b2_user_facing_diagnostics_are_firewall_safe() {
        use crate::compiler::comptime_builtins::trait_evidence::{
            FIND_IMPL_NAMED_IMPLS_ONLY_DIAGNOSTIC, FORGED_IMPL_EVIDENCE_DIAGNOSTIC,
            FORGED_TRAIT_REF_DIAGNOSTIC, TRAIT_NOT_A_VALUE_TYPE_DIAGNOSTIC,
            TRAIT_REF_NOT_A_TRAIT_DIAGNOSTIC,
        };
        use crate::compiler::helpers::{comptime_message_has_jargon, sanitize_comptime_internal};

        for (name, text) in [
            (
                "BLANKET_IMPL_NOT_EVIDENCE_DIAGNOSTIC",
                BLANKET_IMPL_NOT_EVIDENCE_DIAGNOSTIC,
            ),
            (
                "NUMERIC_WIDENING_NOT_EVIDENCE_DIAGNOSTIC",
                NUMERIC_WIDENING_NOT_EVIDENCE_DIAGNOSTIC,
            ),
            (
                "AMBIGUOUS_IMPL_EVIDENCE_DIAGNOSTIC",
                AMBIGUOUS_IMPL_EVIDENCE_DIAGNOSTIC,
            ),
            (
                "POST_BARRIER_IMPL_NOT_EVIDENCE_DIAGNOSTIC",
                POST_BARRIER_IMPL_NOT_EVIDENCE_DIAGNOSTIC,
            ),
            ("FORGED_TRAIT_REF_DIAGNOSTIC", FORGED_TRAIT_REF_DIAGNOSTIC),
            (
                "FORGED_IMPL_EVIDENCE_DIAGNOSTIC",
                FORGED_IMPL_EVIDENCE_DIAGNOSTIC,
            ),
            (
                "TRAIT_REF_NOT_A_TRAIT_DIAGNOSTIC",
                TRAIT_REF_NOT_A_TRAIT_DIAGNOSTIC,
            ),
            (
                "TRAIT_NOT_A_VALUE_TYPE_DIAGNOSTIC",
                TRAIT_NOT_A_VALUE_TYPE_DIAGNOSTIC,
            ),
            (
                "FIND_IMPL_NAMED_IMPLS_ONLY_DIAGNOSTIC",
                FIND_IMPL_NAMED_IMPLS_ONLY_DIAGNOSTIC,
            ),
            (
                "ENUM_HEAD_PARAM_KIND_UNRECOVERABLE_DIAGNOSTIC",
                ENUM_HEAD_PARAM_KIND_UNRECOVERABLE_DIAGNOSTIC,
            ),
            (
                "NOT_A_TYPE_CONSTRUCTOR_DIAGNOSTIC",
                NOT_A_TYPE_CONSTRUCTOR_DIAGNOSTIC,
            ),
        ] {
            assert!(
                !comptime_message_has_jargon(text),
                "{name} carries internal jargon and would be firewall-masked user-facing: {text}"
            );
            assert_eq!(
                sanitize_comptime_internal(text),
                text,
                "{name} must pass the comptime diagnostics firewall unchanged"
            );
        }
    }

    // ========================================================================
    // ADR-009 B4 (Dec 54): the freeze param-kind query — one param-kind
    // projection (arity = vector length), never a second table. Builtins and
    // user struct generics carry frozen kinds; generic enum heads are the
    // named surface-and-stop; non-nominals are the named non-constructor
    // rejection.
    // ========================================================================

    fn type_param(name: &str) -> shape_ast::ast::TypeParam {
        shape_ast::ast::TypeParam::Type {
            name: name.to_string(),
            span: shape_ast::ast::Span::DUMMY,
            doc_comment: None,
            default_type: None,
            trait_bounds: Vec::new(),
        }
    }

    fn const_param(name: &str) -> shape_ast::ast::TypeParam {
        shape_ast::ast::TypeParam::Const {
            name: name.to_string(),
            span: shape_ast::ast::Span::DUMMY,
            doc_comment: None,
            ty: TypeAnnotation::Basic("int".to_string()),
            default: None,
        }
    }

    fn add_struct_with_params(
        compiler: &mut BytecodeCompiler,
        name: &str,
        params: Vec<shape_ast::ast::TypeParam>,
    ) {
        compiler
            .struct_types
            .insert(name.to_string(), (Vec::new(), shape_ast::ast::Span::DUMMY));
        compiler.struct_generic_info.insert(
            name.to_string(),
            crate::compiler::StructGenericInfo {
                type_params: params,
                runtime_field_types: HashMap::new(),
            },
        );
    }

    #[test]
    fn param_kinds_of_projects_builtin_type_parameters() {
        let freeze =
            SemanticFreeze::freeze(&BytecodeCompiler::new()).expect("bare compiler must freeze");

        // Arity = vector length; every builtin generic parameter is a type
        // parameter (no builtin declares a const generic).
        for (name, arity) in [("Array", 1), ("HashMap", 2), ("Option", 1), ("Result", 2)] {
            let identity = freeze.identity_of(name).expect("builtin identity");
            let kinds = freeze
                .param_kinds_of(identity)
                .expect("builtin has frozen kinds");
            assert_eq!(kinds.len(), arity, "{name} arity");
            assert!(
                kinds.iter().all(|kind| *kind == ParamKind::Type),
                "{name} parameters are all type parameters"
            );
        }
    }

    #[test]
    fn param_kinds_of_projects_user_struct_type_and_const_parameters_in_order() {
        let freeze = SemanticFreeze::freeze(&{
            let mut compiler = BytecodeCompiler::new();
            add_struct_with_params(
                &mut compiler,
                "Vector",
                vec![type_param("T"), const_param("N")],
            );
            compiler
        })
        .expect("compiler with generic struct must freeze");

        let identity = freeze.identity_of("Vector").expect("Vector identity");
        assert_eq!(
            freeze
                .param_kinds_of(identity)
                .expect("Vector has frozen kinds"),
            &[ParamKind::Type, ParamKind::Const],
            "ordered kinds project the declared type params, arity = length"
        );
    }

    #[test]
    fn param_kinds_of_surfaces_and_stops_on_generic_enum_heads() {
        // Named freeze input 3: enums enter through the schema registry, which
        // carries no generic arity/kinds — the query surfaces-and-stops with a
        // NAMED diagnostic, never a guessed kind and never a silent None.
        let freeze = SemanticFreeze::freeze(&compiler_with_module_scope_types())
            .expect("module-scope compiler must freeze");
        let color = freeze.identity_of("Color").expect("Color enum identity");
        assert_eq!(freeze.category_of(color), Ok(FrozenTypeCategory::Nominal));
        let error = freeze
            .param_kinds_of(color)
            .expect_err("enum head kinds are unrecoverable — surface-and-stop");
        assert_eq!(error, ENUM_HEAD_PARAM_KIND_UNRECOVERABLE_DIAGNOSTIC);
    }

    #[test]
    fn param_kinds_of_rejects_non_nominal_identities() {
        let freeze =
            SemanticFreeze::freeze(&BytecodeCompiler::new()).expect("bare compiler must freeze");
        let int_identity = freeze.identity_of("int").expect("int identity");
        let error = freeze
            .param_kinds_of(int_identity)
            .expect_err("a primitive is not a type constructor");
        assert_eq!(error, NOT_A_TYPE_CONSTRUCTOR_DIAGNOSTIC);
    }

    #[test]
    fn param_kinds_query_flows_through_the_overlay_unchanged() {
        let freeze = SemanticFreeze::freeze(&{
            let mut compiler = BytecodeCompiler::new();
            add_struct_with_params(&mut compiler, "Box", vec![type_param("T")]);
            compiler
        })
        .expect("compiler with generic struct must freeze");
        // A scoped overlay never shadows a nominal constructor's kinds.
        let overlay = FreezeOverlay::new(freeze, "map", &["T".to_string()]);
        let identity = overlay.identity_of("Box").expect("Box identity");
        assert_eq!(
            overlay
                .param_kinds_of(identity)
                .expect("overlay defers to base"),
            &[ParamKind::Type]
        );
    }
}
