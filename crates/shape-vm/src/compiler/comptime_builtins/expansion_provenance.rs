//! ADR-009 ticket D1 (slice S1) — identity core for generated declarations.
//!
//! Decision 68 (`docs/design/typed-comptime/expansion-and-tooling.md`):
//! every generated declaration is an ordinary checked compiler symbol with
//! stable expansion provenance. Generated text and dummy spans are not
//! semantic representations. This module hosts the identity types
//! ([`SymbolId`], [`ExpansionIdentity`], [`GeneratedOrigin`]) plus the
//! compiler-owned issuing registry ([`GeneratedSymbolTable`]); slice S2 wires
//! them onto the existing extend/materialization path
//! (`functions_annotations.rs::materialize_computed_comptime_extends` /
//! `apply_comptime_extend`).
//!
//! Hashing reuses the A1 canonical-descriptor SHA-256 scheme
//! (`type_reflection.rs::FrozenTypeIdentity::from_canonical_descriptor`):
//! 128 bits of a SHA-256 digest over CANONICAL DESCRIPTOR strings — never
//! rendered source text, never an incrementing counter (counter-allocated
//! identity is the recurring schema-id collision root).

// D1 slice S1 lands the identity core one slice ahead of its consumers: the
// S2 stamping pass (`materialize_computed_comptime_extends` /
// `apply_comptime_extend`) is the first production caller. The allow below
// keeps the canonical check gate warning-clean in the interim and MUST be
// deleted in slice S2 when the surface is consumed.
#![allow(dead_code)]

use sha2::{Digest, Sha256};
use shape_ast::ast::Span;
use std::collections::HashMap;

/// Rejection-matrix row 1 (ticket D1): a generated node without a compiler
/// symbol identity and expansion provenance — including any attempt to anchor
/// it at `Span::DUMMY` — is a compile error, never an unstamped node
/// (Decision 68 required rejection).
pub(crate) const GENERATED_NODE_WITHOUT_PROVENANCE_DIAGNOSTIC: &str =
    "generated nodes require compiler symbol identity and expansion \
     provenance: the source anchor must reference a real span in a real \
     source file, never Span::DUMMY (ADR-009 Decision 68)";

/// Rejection-matrix rows 2/3 (ticket D1): a second, conflicting declaration
/// for an already-reserved expansion identity is a compile error carrying
/// full expansion provenance (Decision 67 invariant 5). Slice S2 attaches
/// the generator/application/target locations at the enforcement point.
pub(crate) const GENERATED_SYMBOL_DUPLICATE_IDENTITY_DIAGNOSTIC: &str =
    "duplicate generated-symbol identity: a conflicting declaration was \
     produced for an already-reserved expansion identity (ADR-009 \
     Decision 67)";

/// Slice S4 rule (declared with the registry so no interim lookup can adopt
/// a weaker contract): an unknown [`SymbolId`] lookup is a named error —
/// surface-and-stop, never a silent absent-value return.
pub(crate) const UNKNOWN_GENERATED_SYMBOL_DIAGNOSTIC: &str =
    "unknown generated-symbol identity: no expansion provenance was \
     registered for the requested SymbolId (ADR-009 Decision 68)";

// Domain-separation tags: each hash kind digests a distinct domain prefix so
// canonically-equal descriptor lists in different roles can never collide.
const ARGUMENTS_HASH_DOMAIN: &str = "adr009:d1:arguments";
const DEPENDENCIES_HASH_DOMAIN: &str = "adr009:d1:dependencies";
const EXPANSION_IDENTITY_DOMAIN: &str = "adr009:d1:expansion-identity";
const SYMBOL_ID_DOMAIN: &str = "adr009:d1:symbol-id";

/// 128-bit canonical-descriptor fingerprint — the same scheme as A1's
/// `FrozenTypeIdentity::from_canonical_descriptor` (128 bits of a SHA-256
/// digest), extended with length-prefix framing so descriptor-list inputs
/// cannot collide by concatenation. Inputs are CANONICAL DESCRIPTORS only:
/// rendered source text never enters a hash (spec §3 invariant 3, rejection
/// row 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct CanonicalHash {
    pub(crate) high: i64,
    pub(crate) low: i64,
}

impl CanonicalHash {
    /// Digest a domain tag plus framed canonical-descriptor parts into the
    /// 128-bit fold used by `FrozenTypeIdentity`.
    fn over_framed<'a>(domain: &str, parts: impl IntoIterator<Item = &'a str>) -> Self {
        let mut hasher = Sha256::new();
        hasher.update((domain.len() as u64).to_be_bytes());
        hasher.update(domain.as_bytes());
        for part in parts {
            hasher.update((part.len() as u64).to_be_bytes());
            hasher.update(part.as_bytes());
        }
        let digest = hasher.finalize();
        let high = i64::from_be_bytes(digest[0..8].try_into().expect("8-byte hash prefix"));
        let low = i64::from_be_bytes(digest[8..16].try_into().expect("8-byte hash suffix"));
        Self { high, low }
    }

    /// Hash a named-argument descriptor SET. Arguments are canonically
    /// ordered by argument name before digesting, so source ordering and
    /// formatting cannot perturb the identity; upstream canonicalization
    /// guarantees argument names are unique within one application.
    pub(crate) fn from_canonical_argument_descriptors(descriptors: &[(&str, &str)]) -> Self {
        let mut sorted: Vec<(&str, &str)> = descriptors.to_vec();
        sorted.sort_unstable();
        Self::over_framed(
            ARGUMENTS_HASH_DOMAIN,
            sorted.iter().flat_map(|(name, descriptor)| [*name, *descriptor]),
        )
    }

    /// Hash a dependency descriptor SET (canonically sorted — dependency
    /// discovery order cannot perturb the identity).
    pub(crate) fn from_canonical_dependency_descriptors(descriptors: &[&str]) -> Self {
        let mut sorted: Vec<&str> = descriptors.to_vec();
        sorted.sort_unstable();
        Self::over_framed(DEPENDENCIES_HASH_DOMAIN, sorted.iter().copied())
    }

    /// Canonical hex rendering, used when a hash participates in a further
    /// canonical descriptor (e.g. the expansion-identity fingerprint).
    fn canonical_descriptor(&self) -> String {
        format!("{:016x}{:016x}", self.high as u64, self.low as u64)
    }
}

/// Canonical reference to the generator definition (the annotation comptime
/// handler / comptime construct that produced the expansion). Built from a
/// canonical descriptor, never from rendered source text or a bare name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct GeneratorRef {
    canonical_descriptor: String,
}

impl GeneratorRef {
    pub(crate) fn from_canonical_descriptor(descriptor: impl Into<String>) -> Self {
        Self {
            canonical_descriptor: descriptor.into(),
        }
    }

    pub(crate) fn canonical_descriptor(&self) -> &str {
        &self.canonical_descriptor
    }
}

/// Canonical identity of one application site (the annotation application /
/// comptime construct the user wrote). The SAME application must produce the
/// SAME `ApplicationId` in the speculative pre-pass and the authoritative
/// pass-2 compile, or identity-keyed dedup breaks.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ApplicationId {
    canonical_descriptor: String,
}

impl ApplicationId {
    pub(crate) fn from_canonical_descriptor(descriptor: impl Into<String>) -> Self {
        Self {
            canonical_descriptor: descriptor.into(),
        }
    }

    pub(crate) fn canonical_descriptor(&self) -> &str {
        &self.canonical_descriptor
    }
}

/// Canonical identity of the expansion target (the type/function/module the
/// generated declarations attach to).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct TargetIdentity {
    canonical_descriptor: String,
}

impl TargetIdentity {
    pub(crate) fn from_canonical_descriptor(descriptor: impl Into<String>) -> Self {
        Self {
            canonical_descriptor: descriptor.into(),
        }
    }

    pub(crate) fn canonical_descriptor(&self) -> &str {
        &self.canonical_descriptor
    }
}

/// The comptime stage an expansion runs at, for the EXISTING
/// extend/materialization path (ticket D1 scope). The declaration-discovery
/// fixed point (Decision 67) is ticket D2; when it lands, its stages extend
/// this enum — D1 deliberately models only the stages the current path
/// actually executes. NOTE: the speculative pre-pass and the authoritative
/// pass-2 run of one application are the SAME stage (same identity), never
/// distinct variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ComptimeStage {
    /// An annotation `@comptime` handler running against its annotated
    /// type/function target (`ComptimeDirective::Extend` / `ExtendItems`).
    AnnotationHandler,
    /// A module-level `comptime {}` block emitting module-scope directives
    /// (`process_comptime_directives_for_module`).
    ModuleComptimeBlock,
}

impl ComptimeStage {
    fn canonical_descriptor(&self) -> &'static str {
        match self {
            Self::AnnotationHandler => "stage:annotation-handler",
            Self::ModuleComptimeBlock => "stage:module-comptime-block",
        }
    }
}

/// Decision 68 expansion identity: the six-component identity of one
/// generator application. All components are required at construction —
/// there is no partial or empty identity (spec §3 invariant 1; A1 row-9
/// structural pattern).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ExpansionIdentity {
    pub(crate) generator: GeneratorRef,
    pub(crate) application: ApplicationId,
    pub(crate) target: TargetIdentity,
    pub(crate) stage: ComptimeStage,
    pub(crate) arguments_hash: CanonicalHash,
    pub(crate) dependencies_hash: CanonicalHash,
}

impl ExpansionIdentity {
    pub(crate) fn new(
        generator: GeneratorRef,
        application: ApplicationId,
        target: TargetIdentity,
        stage: ComptimeStage,
        arguments_hash: CanonicalHash,
        dependencies_hash: CanonicalHash,
    ) -> Self {
        Self {
            generator,
            application,
            target,
            stage,
            arguments_hash,
            dependencies_hash,
        }
    }

    /// Content fingerprint over the six components' canonical descriptors.
    /// Sensitive to every component; insensitive to anything not in the
    /// canonical descriptors (formatting, discovery order, rendered text).
    pub(crate) fn fingerprint(&self) -> CanonicalHash {
        CanonicalHash::over_framed(
            EXPANSION_IDENTITY_DOMAIN,
            [
                self.generator.canonical_descriptor(),
                self.application.canonical_descriptor(),
                self.target.canonical_descriptor(),
                self.stage.canonical_descriptor(),
                &self.arguments_hash.canonical_descriptor(),
                &self.dependencies_hash.canonical_descriptor(),
            ],
        )
    }
}

/// Opaque compiler-issued identity of one generated declaration.
///
/// Content-derived: 128 bits of SHA-256 over the owning
/// [`ExpansionIdentity`] fingerprint plus the declaration-name canonical
/// descriptor — never a counter (the schema-id collision family), never raw
/// name text. The constructor is private to this module (ProofGap pattern,
/// CLAUDE.md §Mechanical enforcement): the only way to obtain a `SymbolId`
/// is to register full provenance with the [`GeneratedSymbolTable`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct SymbolId {
    high: i64,
    low: i64,
}

impl SymbolId {
    /// Private to the issuing module: emit code cannot fabricate a
    /// `SymbolId` without registering provenance.
    fn derive(expansion: &ExpansionIdentity, decl_name_descriptor: &str) -> Self {
        let fingerprint = expansion.fingerprint().canonical_descriptor();
        let hash = CanonicalHash::over_framed(
            SYMBOL_ID_DOMAIN,
            [fingerprint.as_str(), decl_name_descriptor],
        );
        Self {
            high: hash.high,
            low: hash.low,
        }
    }
}

/// Structured path from a generated declaration's root to one generated
/// node. Non-empty by construction: a path always starts at a declaration
/// root segment.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct GeneratedNodePath {
    segments: Vec<String>,
}

impl GeneratedNodePath {
    /// Start a path at the generated declaration's root segment.
    pub(crate) fn decl_root(segment: impl Into<String>) -> Self {
        Self {
            segments: vec![segment.into()],
        }
    }

    /// Extend the path with a child node segment.
    pub(crate) fn child(&self, segment: impl Into<String>) -> Self {
        let mut segments = self.segments.clone();
        segments.push(segment.into());
        Self { segments }
    }

    pub(crate) fn segments(&self) -> &[String] {
        &self.segments
    }
}

/// A REAL source location: a `Span` paired with the `SourceMap` file
/// identity (`bytecode/core_types.rs`, u16 file ids) it indexes into. Both
/// components are required — an anchorless generated node is structurally
/// impossible, and `Span::DUMMY` is rejected at construction with the named
/// row-1 diagnostic. A genuine zero-offset span (start 0, end > 0) is a
/// legitimate anchor; only the empty `{0, 0}` dummy is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct SourceAnchor {
    file_id: u16,
    span: Span,
}

impl SourceAnchor {
    pub(crate) fn new(file_id: u16, span: Span) -> Result<Self, String> {
        if span == Span::DUMMY {
            return Err(GENERATED_NODE_WITHOUT_PROVENANCE_DIAGNOSTIC.to_string());
        }
        Ok(Self { file_id, span })
    }

    pub(crate) fn file_id(&self) -> u16 {
        self.file_id
    }

    pub(crate) fn span(&self) -> Span {
        self.span
    }
}

/// Decision 68 generated-node origin: the expansion that produced the node,
/// the structured path to it, and its real source anchor. All fields are
/// required (no partial provenance).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct GeneratedOrigin {
    pub(crate) expansion: ExpansionIdentity,
    pub(crate) node_path: GeneratedNodePath,
    pub(crate) source_anchor: SourceAnchor,
}

/// Compiler-owned registry of generated-symbol identities — the SINGLE
/// source of truth for "this declaration was generated, by whom, from
/// where". The name-keyed `materialized_comptime_fns` set becomes a derived
/// lookup INTO this table in slice S2, never an identity of its own.
///
/// Lives on `BytecodeCompiler` (`compiler/mod.rs`), initialized empty per
/// compilation unit beside the A3 specialization overlay.
#[derive(Debug)]
pub(crate) struct GeneratedSymbolTable {
    records: HashMap<SymbolId, GeneratedOrigin>,
}

impl GeneratedSymbolTable {
    /// One empty registry per compilation unit. (An empty TABLE is a
    /// legitimate starting state; the identity TYPES it stores have no
    /// empty form.)
    pub(crate) fn new() -> Self {
        Self {
            records: HashMap::new(),
        }
    }

    /// Issue the compiler symbol identity for a generated declaration by
    /// registering its full provenance. Re-issuing the SAME identity with
    /// the SAME origin is idempotent (the speculative pre-pass and the
    /// authoritative pass-2 compile agree on one identity); a CONFLICTING
    /// origin for a reserved identity is the named duplicate-identity
    /// error (slice S2 raises it as a compile error with generator +
    /// application + target locations).
    pub(crate) fn issue(
        &mut self,
        decl_name_descriptor: &str,
        origin: GeneratedOrigin,
    ) -> Result<SymbolId, String> {
        let id = SymbolId::derive(&origin.expansion, decl_name_descriptor);
        if let Some(existing) = self.records.get(&id) {
            if *existing != origin {
                return Err(GENERATED_SYMBOL_DUPLICATE_IDENTITY_DIAGNOSTIC.to_string());
            }
            return Ok(id);
        }
        self.records.insert(id, origin);
        Ok(id)
    }

    /// Provenance lookup. An unknown identity is a named error — never a
    /// silent absent-value return (surface-and-stop).
    pub(crate) fn origin_of(&self, id: SymbolId) -> Result<&GeneratedOrigin, String> {
        self.records
            .get(&id)
            .ok_or_else(|| UNKNOWN_GENERATED_SYMBOL_DIAGNOSTIC.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::ast::Span;

    fn sample_arguments_hash() -> CanonicalHash {
        CanonicalHash::from_canonical_argument_descriptors(&[
            ("debug", "bool=true"),
            ("name", "string=schema"),
        ])
    }

    fn sample_dependencies_hash() -> CanonicalHash {
        CanonicalHash::from_canonical_dependency_descriptors(&[
            "type:app.main:Point",
            "type:app.main:Vec2",
        ])
    }

    fn sample_identity() -> ExpansionIdentity {
        ExpansionIdentity::new(
            GeneratorRef::from_canonical_descriptor("annotation:json_schema@app.main"),
            ApplicationId::from_canonical_descriptor("application:app.main:Point:json_schema"),
            TargetIdentity::from_canonical_descriptor("type:app.main:Point"),
            ComptimeStage::AnnotationHandler,
            sample_arguments_hash(),
            sample_dependencies_hash(),
        )
    }

    fn sample_origin() -> GeneratedOrigin {
        GeneratedOrigin {
            expansion: sample_identity(),
            node_path: GeneratedNodePath::decl_root("extend:Point").child("method:sum"),
            source_anchor: SourceAnchor::new(0, Span::new(42, 57))
                .expect("real span must anchor"),
        }
    }

    // (a) Hash determinism: two independent constructions from the same
    // canonical descriptors agree on identity, fingerprint, and issued
    // SymbolId (the pre-pass/pass-2 agreement precondition).
    #[test]
    fn expansion_identity_is_deterministic_across_constructions() {
        let first = sample_identity();
        let second = sample_identity();
        assert_eq!(first, second);
        assert_eq!(first.fingerprint(), second.fingerprint());

        let mut table = GeneratedSymbolTable::new();
        let origin_a = sample_origin();
        let origin_b = sample_origin();
        let id_a = table
            .issue("method:Point.sum", origin_a)
            .expect("first issue succeeds");
        // Re-issuing the SAME identity + origin (speculative pre-pass then
        // authoritative pass-2) is idempotent: same SymbolId, no conflict.
        let id_b = table
            .issue("method:Point.sum", origin_b)
            .expect("idempotent re-issue succeeds");
        assert_eq!(id_a, id_b);
    }

    // (b) Sensitivity: changing ANY of the six ExpansionIdentity components
    // changes the fingerprint (and hence any SymbolId derived from it).
    #[test]
    fn expansion_identity_fingerprint_is_sensitive_to_every_component() {
        let base = sample_identity();

        let variants = [
            ExpansionIdentity::new(
                GeneratorRef::from_canonical_descriptor("annotation:display@app.main"),
                base.application.clone(),
                base.target.clone(),
                base.stage,
                base.arguments_hash,
                base.dependencies_hash,
            ),
            ExpansionIdentity::new(
                base.generator.clone(),
                ApplicationId::from_canonical_descriptor("application:app.main:Vec2:json_schema"),
                base.target.clone(),
                base.stage,
                base.arguments_hash,
                base.dependencies_hash,
            ),
            ExpansionIdentity::new(
                base.generator.clone(),
                base.application.clone(),
                TargetIdentity::from_canonical_descriptor("type:app.main:Vec2"),
                base.stage,
                base.arguments_hash,
                base.dependencies_hash,
            ),
            ExpansionIdentity::new(
                base.generator.clone(),
                base.application.clone(),
                base.target.clone(),
                ComptimeStage::ModuleComptimeBlock,
                base.arguments_hash,
                base.dependencies_hash,
            ),
            ExpansionIdentity::new(
                base.generator.clone(),
                base.application.clone(),
                base.target.clone(),
                base.stage,
                CanonicalHash::from_canonical_argument_descriptors(&[("debug", "bool=false")]),
                base.dependencies_hash,
            ),
            ExpansionIdentity::new(
                base.generator.clone(),
                base.application.clone(),
                base.target.clone(),
                base.stage,
                base.arguments_hash,
                CanonicalHash::from_canonical_dependency_descriptors(&["type:app.main:Other"]),
            ),
        ];

        for (i, variant) in variants.iter().enumerate() {
            assert_ne!(
                base.fingerprint(),
                variant.fingerprint(),
                "component {i} did not perturb the fingerprint"
            );
        }
    }

    // (b, SymbolId axis) the declaration-name descriptor also distinguishes
    // symbols issued under one expansion identity.
    #[test]
    fn symbol_id_is_sensitive_to_decl_name_descriptor() {
        let mut table = GeneratedSymbolTable::new();
        let sum = table
            .issue("method:Point.sum", sample_origin())
            .expect("issue sum");
        let scale = table
            .issue(
                "method:Point.scale",
                GeneratedOrigin {
                    expansion: sample_identity(),
                    node_path: GeneratedNodePath::decl_root("extend:Point").child("method:scale"),
                    source_anchor: SourceAnchor::new(0, Span::new(42, 57))
                        .expect("real span must anchor"),
                },
            )
            .expect("issue scale");
        assert_ne!(sum, scale);
    }

    // (c) Formatting-insensitivity: arguments_hash / dependencies_hash are
    // built from canonical descriptor SETS — differently-formatted (here:
    // differently source-ordered) but semantically equal argument sets hash
    // identically, and rendered source text never enters the hash (rejection
    // row 8: the constructors accept canonical descriptors only).
    #[test]
    fn argument_and_dependency_hashes_are_formatting_insensitive() {
        let source_order = CanonicalHash::from_canonical_argument_descriptors(&[
            ("debug", "bool=true"),
            ("name", "string=schema"),
        ]);
        let swapped_order = CanonicalHash::from_canonical_argument_descriptors(&[
            ("name", "string=schema"),
            ("debug", "bool=true"),
        ]);
        assert_eq!(source_order, swapped_order);

        let deps = CanonicalHash::from_canonical_dependency_descriptors(&[
            "type:app.main:Point",
            "type:app.main:Vec2",
        ]);
        let deps_swapped = CanonicalHash::from_canonical_dependency_descriptors(&[
            "type:app.main:Vec2",
            "type:app.main:Point",
        ]);
        assert_eq!(deps, deps_swapped);
    }

    // (c, framing) descriptor framing is unambiguous: ["a","bc"] and
    // ["ab","c"] must not concatenate to the same digest input.
    #[test]
    fn descriptor_framing_prevents_concatenation_collisions() {
        let split_one = CanonicalHash::from_canonical_dependency_descriptors(&["a", "bc"]);
        let split_two = CanonicalHash::from_canonical_dependency_descriptors(&["ab", "c"]);
        assert_ne!(split_one, split_two);

        let arg_split_one =
            CanonicalHash::from_canonical_argument_descriptors(&[("a", "bc")]);
        let arg_split_two =
            CanonicalHash::from_canonical_argument_descriptors(&[("ab", "c")]);
        assert_ne!(arg_split_one, arg_split_two);
    }

    // Source anchors are REAL locations: Span::DUMMY is rejected at the
    // constructor (the module's own structural invariant), while a genuine
    // zero-offset span (start 0, end > 0) is a legitimate anchor — the
    // DUMMY-vs-offset-0 distinction.
    #[test]
    fn source_anchor_rejects_dummy_span_but_accepts_offset_zero() {
        let err = SourceAnchor::new(0, Span::DUMMY).expect_err("DUMMY must be rejected");
        assert!(
            err.contains(GENERATED_NODE_WITHOUT_PROVENANCE_DIAGNOSTIC),
            "rejection must carry the named diagnostic, got: {err}"
        );
        let anchored = SourceAnchor::new(3, Span::new(0, 12)).expect("offset-0 span is real");
        assert_eq!(anchored.file_id(), 3);
        assert_eq!(anchored.span(), Span::new(0, 12));
    }

    // (d) Row-9 structural pattern, A1 precedent: the three identity types
    // are total by construction — no zero-value impl or derive, no
    // empty/partial constructor, no optional fields. The compile-time part
    // is enforced by construction (every constructor requires every
    // component; `GeneratedOrigin` has no constructor other than the full
    // struct literal). This diff-review grep note pins the source shape.
    #[test]
    fn identity_core_row9_structural_grep_note() {
        let src = include_str!("expansion_provenance.rs");
        // Needles assembled by concatenation so this test's own source
        // cannot satisfy them. The whole file (impls, comments, tests) must
        // stay clean of these tokens.
        let default_token = ["Def", "ault"].concat();
        assert!(
            !src.contains(&default_token),
            "no zero-value impl/derive may appear in the identity core"
        );
        let option_token = ["Opt", "ion<"].concat();
        assert!(
            !src.contains(&option_token),
            "no optional fields/returns may appear in the identity core"
        );
        let counter_needle = ["next", "_id"].concat();
        assert!(
            !src.contains(&counter_needle),
            "identity is content-derived, never counter-allocated"
        );
    }
}
