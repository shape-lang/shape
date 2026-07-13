//! ADR-009 ticket D1 (slice S1) — identity core for generated declarations.
//!
//! Decision 68 (`docs/design/typed-comptime/expansion-and-tooling.md`):
//! every generated declaration is an ordinary checked compiler symbol with
//! stable expansion provenance. Generated text and dummy spans are not
//! semantic representations. This module hosts the identity types
//! ([`SymbolId`], [`ExpansionIdentity`], [`GeneratedOrigin`]) plus the
//! compiler-owned issuing registry ([`GeneratedSymbolTable`]). Slice S2
//! stamps them onto the existing two-phase extend/materialization path
//! (`functions_annotations.rs::materialize_computed_comptime_extends` /
//! `apply_comptime_extend` / `apply_comptime_extend_items`): every
//! directive-producing site builds an [`ExpansionSite`], and every generated
//! declaration is reserved in the table before registration.
//!
//! Hashing reuses the A1 canonical-descriptor SHA-256 scheme
//! (`type_reflection.rs::FrozenTypeIdentity::from_canonical_descriptor`):
//! 128 bits of a SHA-256 digest over CANONICAL DESCRIPTOR strings — never
//! rendered source text, never an incrementing counter (counter-allocated
//! identity is the recurring schema-id collision root).

use sha2::{Digest, Sha256};
use shape_ast::ast::Span;
use std::collections::HashMap;

/// Rejection-matrix row 1 (ticket D1): a generated node without a compiler
/// symbol identity and expansion provenance — including any attempt to anchor
/// it at `Span::DUMMY` — is a compile error, never an unstamped node
/// (Decision 68 required rejection).
pub(crate) const GENERATED_NODE_WITHOUT_PROVENANCE_DIAGNOSTIC: &str = "generated nodes require compiler symbol identity and expansion \
     provenance: the source anchor must reference a real span in a real \
     source file, never Span::DUMMY (ADR-009 Decision 68)";

/// Rejection-matrix row 3 (ticket D1): the same full application identity
/// expanded twice with CONFLICTING output (same reserved identity, different
/// declaration content or origin) is a compile error carrying full expansion
/// provenance (Decision 67 invariant 2/5).
pub(crate) const GENERATED_SYMBOL_DUPLICATE_IDENTITY_DIAGNOSTIC: &str = "duplicate generated-symbol identity: a conflicting declaration was \
     produced for an already-reserved expansion identity (ADR-009 \
     Decision 67)";

/// Rejection-matrix row 2 (ticket D1): one generated declaration NAME
/// reserved by two DIFFERENT expansion identities (e.g. two annotation
/// applications each generating `fn clash()`) is a compile error carrying
/// both expansions' provenance — never a silent first-wins dedup (Decision
/// 67 invariant 5).
pub(crate) const GENERATED_SYMBOL_CONFLICT_DIAGNOSTIC: &str = "conflicting generated-symbol definition: this declaration name is \
     already reserved by a different expansion identity (ADR-009 \
     Decision 67)";

/// Slice S4 rule (declared with the registry so no interim lookup can adopt
/// a weaker contract): an unknown [`SymbolId`] lookup is a named error —
/// surface-and-stop, never a silent absent-value return.
pub(crate) const UNKNOWN_GENERATED_SYMBOL_DIAGNOSTIC: &str = "unknown generated-symbol identity: no expansion provenance was \
     registered for the requested SymbolId (ADR-009 Decision 68)";

// Domain-separation tags: each hash kind digests a distinct domain prefix so
// canonically-equal descriptor lists in different roles can never collide.
const ARGUMENTS_HASH_DOMAIN: &str = "adr009:d1:arguments";
const DEPENDENCIES_HASH_DOMAIN: &str = "adr009:d1:dependencies";
const EXPANSION_IDENTITY_DOMAIN: &str = "adr009:d1:expansion-identity";
const SYMBOL_ID_DOMAIN: &str = "adr009:d1:symbol-id";
const GENERATED_CONTENT_DOMAIN: &str = "adr009:d1:generated-content";

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
            sorted
                .iter()
                .flat_map(|(name, descriptor)| [*name, *descriptor]),
        )
    }

    /// Hash a dependency descriptor SET (canonically sorted — dependency
    /// discovery order cannot perturb the identity).
    pub(crate) fn from_canonical_dependency_descriptors(descriptors: &[&str]) -> Self {
        let mut sorted: Vec<&str> = descriptors.to_vec();
        sorted.sort_unstable();
        Self::over_framed(DEPENDENCIES_HASH_DOMAIN, sorted.iter().copied())
    }

    /// Hash the canonical structural AST encoding of one generated
    /// declaration (rejection row 3's "conflicting output" detector). The
    /// encoding is a structural fold over the emitted declaration AST —
    /// never rendered source text (spec §3 invariant 3): equal handler
    /// output parses to an equal encoding across the speculative pre-pass
    /// and the authoritative pass-2 run.
    pub(crate) fn from_canonical_decl_encoding(encoding: &str) -> Self {
        Self::over_framed(GENERATED_CONTENT_DOMAIN, [encoding])
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
pub struct SymbolId {
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
pub struct GeneratedNodePath {
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

    pub fn segments(&self) -> &[String] {
        &self.segments
    }

    /// Canonical `root/child/…` rendering for diagnostics (S4 row 7: the
    /// generated-node note names the node path so a body error inside a
    /// generated declaration is attributable without virtual documents,
    /// which are ticket D2).
    pub fn render(&self) -> String {
        self.segments.join("/")
    }
}

/// A REAL source location: a `Span` paired with the `SourceMap` file
/// identity (`bytecode/core_types.rs`, u16 file ids) it indexes into. Both
/// components are required — an anchorless generated node is structurally
/// impossible, and `Span::DUMMY` is rejected at construction with the named
/// row-1 diagnostic. A genuine zero-offset span (start 0, end > 0) is a
/// legitimate anchor; only the empty `{0, 0}` dummy is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceAnchor {
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

    pub fn file_id(&self) -> u16 {
        self.file_id
    }

    pub fn span(&self) -> Span {
        self.span
    }

    /// Does this anchor's span contain byte `offset` in file `file_id`?
    /// (Half-open `[start, end)` containment — the position-resolution
    /// predicate of the S4 query surface.)
    pub fn contains(&self, file_id: u16, offset: usize) -> bool {
        self.file_id == file_id && self.span.start <= offset && offset < self.span.end
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

impl GeneratedOrigin {
    /// Human-readable provenance line for compile errors: generator,
    /// application, target, and the real source anchor. Diagnostics-grade
    /// LSDS threading (three related locations) is slice S4; the named
    /// rejection errors carry this rendering so provenance is never lost.
    pub(crate) fn describe(&self) -> String {
        format!(
            "generator `{}` applied at `{}` on `{}` (anchored at file #{} span {}..{})",
            self.expansion.generator.canonical_descriptor(),
            self.expansion.application.canonical_descriptor(),
            self.expansion.target.canonical_descriptor(),
            self.source_anchor.file_id(),
            self.source_anchor.span().start,
            self.source_anchor.span().end,
        )
    }
}

/// One comptime expansion SITE: the six-component identity plus the raw
/// application anchor (SourceMap file id + application span) and the raw
/// generator-definition anchor (the annotation `comptime` handler's span;
/// for a `comptime { }` block, the block IS its own generator, so both
/// spans coincide). Built once at each directive-producing site
/// (`annotation_expansion_site` / `comptime_block_expansion_site` in
/// `functions_annotations.rs`) and handed to the directive-consumption
/// points; the anchors are validated into [`SourceAnchor`]s at the
/// registration entry point, where a dummy span is the named row-1 compile
/// error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExpansionSite {
    identity: ExpansionIdentity,
    file_id: u16,
    application_span: Span,
    generator_span: Span,
}

impl ExpansionSite {
    pub(crate) fn new(
        identity: ExpansionIdentity,
        file_id: u16,
        application_span: Span,
        generator_span: Span,
    ) -> Self {
        Self {
            identity,
            file_id,
            application_span,
            generator_span,
        }
    }

    pub(crate) fn identity(&self) -> &ExpansionIdentity {
        &self.identity
    }

    pub(crate) fn application_span(&self) -> Span {
        self.application_span
    }

    pub(crate) fn generator_span(&self) -> Span {
        self.generator_span
    }

    /// Validate the application anchor into a real [`SourceAnchor`].
    /// Rejection row 1: a dummy application span cannot anchor a generated
    /// declaration — named compile error, surface-and-stop.
    pub(crate) fn source_anchor(&self) -> Result<SourceAnchor, String> {
        SourceAnchor::new(self.file_id, self.application_span)
    }

    /// Validate the generator-definition anchor into a real
    /// [`SourceAnchor`]. Same row-1 rule: a generated declaration whose
    /// generator cannot be located in real source has incomplete
    /// provenance — named compile error, surface-and-stop.
    pub(crate) fn generator_anchor(&self) -> Result<SourceAnchor, String> {
        SourceAnchor::new(self.file_id, self.generator_span)
    }
}

/// One reserved generated declaration: its full origin, the canonical
/// content fingerprint of the emitted declaration (rejection row 3), the
/// declaration's function-table name, and the generator-definition anchor
/// (the S4 query surface answers "generator defined here" from the table,
/// never from a text scan).
#[derive(Debug, Clone, PartialEq, Eq)]
struct GeneratedSymbolRecord {
    origin: GeneratedOrigin,
    content: CanonicalHash,
    decl_name: String,
    generator_anchor: SourceAnchor,
}

/// One generated symbol's full provenance, as answered by the S4 query
/// surface (Decision 66: the LSP consumes compiler query results — never a
/// text scan, never a second evaluation). All locations are real
/// [`SourceAnchor`]s by construction.
///
/// `checked_decl` is where the CHECKED generated declaration anchors in
/// real source. Until ticket D2's `shape-expansion://` virtual documents
/// give checked declarations their own addressable text, the S3 anchoring
/// rule places every generated decl at its application span, so
/// `checked_decl` and `application` carry the same anchor — two roles, one
/// location today; the field split is the Decision 68 query shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedSymbolProvenance<'a> {
    pub symbol: SymbolId,
    pub decl_name: &'a str,
    pub node_path: &'a GeneratedNodePath,
    pub checked_decl: SourceAnchor,
    pub application: SourceAnchor,
    pub generator: SourceAnchor,
}

/// Outcome of reserving a generated declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SymbolReservation {
    /// First reservation of this identity: the caller must register the
    /// declaration as an ordinary compiler symbol.
    Fresh(SymbolId),
    /// The SAME identity + origin + content was already reserved (the
    /// speculative pre-pass and the authoritative pass-2 compile agree on
    /// one identity): skip re-registration, compile the body into the
    /// already-registered slot.
    Reissued(SymbolId),
}

/// Compiler-owned registry of generated-symbol identities — the SINGLE
/// source of truth for "this declaration was generated, by whom, from
/// where". The formerly name-keyed `materialized_comptime_fns` set is
/// deleted in slice S2: name lookups (`contains_name` / `symbol_for_name`)
/// are a derived view INTO this table, never an identity of their own.
///
/// Lives on `BytecodeCompiler` (`compiler/mod.rs`), initialized empty per
/// compilation unit beside the A3 specialization overlay. Exposed to
/// tooling via `BytecodeCompiler::generated_symbol_query()` — the ONE
/// query API of spec §4.1 (slice S5 wires LSP navigation onto it).
#[derive(Debug)]
pub struct GeneratedSymbolTable {
    records: HashMap<SymbolId, GeneratedSymbolRecord>,
    /// Derived name index: generated function-table name -> issued
    /// SymbolId. A lookup convenience only — identity lives in `records`.
    names: HashMap<String, SymbolId>,
}

impl GeneratedSymbolTable {
    /// One empty registry per compilation unit. (An empty TABLE is a
    /// legitimate starting state; the identity TYPES it stores have no
    /// empty form.)
    pub(crate) fn new() -> Self {
        Self {
            records: HashMap::new(),
            names: HashMap::new(),
        }
    }

    /// Reserve the compiler symbol identity for a generated declaration by
    /// registering its full provenance and content fingerprint.
    ///
    /// - Fresh identity -> `Fresh` (caller registers the declaration).
    /// - Same identity, same origin + content -> `Reissued` (pre-pass and
    ///   pass-2 agree; idempotent).
    /// - Same identity, conflicting origin or content -> named row-3
    ///   duplicate-identity error (Decision 67 invariant 2/5).
    /// - Same declaration NAME under a different identity -> named row-2
    ///   conflict error carrying both expansions' provenance.
    pub(crate) fn reserve_generated_decl(
        &mut self,
        decl_name: &str,
        origin: GeneratedOrigin,
        content: CanonicalHash,
        generator_anchor: SourceAnchor,
    ) -> Result<SymbolReservation, String> {
        let id = SymbolId::derive(&origin.expansion, decl_name);
        if let Some(existing_id) = self.names.get(decl_name) {
            if *existing_id != id {
                let existing = &self.records[existing_id];
                return Err(format!(
                    "{GENERATED_SYMBOL_CONFLICT_DIAGNOSTIC}: `{decl_name}` was first \
                     generated by {}; the conflicting definition comes from {}",
                    existing.origin.describe(),
                    origin.describe(),
                ));
            }
        }
        if let Some(existing) = self.records.get(&id) {
            if existing.origin != origin || existing.content != content {
                return Err(format!(
                    "{GENERATED_SYMBOL_DUPLICATE_IDENTITY_DIAGNOSTIC}: `{decl_name}` from {} \
                     was expanded again with conflicting output",
                    origin.describe(),
                ));
            }
            return Ok(SymbolReservation::Reissued(id));
        }
        self.records.insert(
            id,
            GeneratedSymbolRecord {
                origin,
                content,
                decl_name: decl_name.to_string(),
                generator_anchor,
            },
        );
        self.names.insert(decl_name.to_string(), id);
        Ok(SymbolReservation::Fresh(id))
    }

    fn provenance_view<'a>(
        &self,
        id: SymbolId,
        record: &'a GeneratedSymbolRecord,
    ) -> GeneratedSymbolProvenance<'a> {
        GeneratedSymbolProvenance {
            symbol: id,
            decl_name: &record.decl_name,
            node_path: &record.origin.node_path,
            checked_decl: record.origin.source_anchor,
            application: record.origin.source_anchor,
            generator: record.generator_anchor,
        }
    }

    /// Full-`GeneratedOrigin` lookup for test assertions on the expansion
    /// identity (the production query surface is [`Self::provenance_of`],
    /// which answers the Decision-68 location shape). An unknown identity
    /// is a named error — never a silent absent-value return
    /// (surface-and-stop).
    #[cfg(test)]
    pub(crate) fn origin_of(&self, id: SymbolId) -> Result<&GeneratedOrigin, String> {
        self.records
            .get(&id)
            .map(|record| &record.origin)
            .ok_or_else(|| UNKNOWN_GENERATED_SYMBOL_DIAGNOSTIC.to_string())
    }

    /// S4 query surface: resolve an issued [`SymbolId`] to its full
    /// provenance — {symbol, checked-decl location, application location,
    /// generator-definition location}. An unknown identity is a named
    /// error — never a silent absent-value return (surface-and-stop).
    pub fn provenance_of(&self, id: SymbolId) -> Result<GeneratedSymbolProvenance<'_>, String> {
        self.records
            .get(&id)
            .map(|record| self.provenance_view(id, record))
            .ok_or_else(|| UNKNOWN_GENERATED_SYMBOL_DIAGNOSTIC.to_string())
    }

    /// S4 query surface: resolve a generated declaration's function-table
    /// name to its full provenance. An unknown name is a named error.
    pub fn provenance_for_name(
        &self,
        decl_name: &str,
    ) -> Result<GeneratedSymbolProvenance<'_>, String> {
        self.provenance_of(self.symbol_for_name(decl_name)?)
    }

    /// S4 query surface: every generated symbol with its provenance, in
    /// deterministic (declaration-name) order — the workspace-symbol
    /// listing (Decision 68 LSP behavior: workspace symbols include
    /// generated symbols via `SymbolId`, answered from this table only).
    pub fn generated_symbols(&self) -> Vec<GeneratedSymbolProvenance<'_>> {
        let mut all: Vec<GeneratedSymbolProvenance<'_>> = self
            .records
            .iter()
            .map(|(id, record)| self.provenance_view(*id, record))
            .collect();
        all.sort_by(|a, b| a.decl_name.cmp(b.decl_name));
        all
    }

    /// S4 query surface: the generated symbols whose CHECKED-DECLARATION
    /// anchor contains byte `offset` of file `file_id` (position → symbol
    /// resolution for navigation). This is a position FILTER over the
    /// table, not an identity lookup: an empty result means "no generated
    /// declaration anchors here" and is a legitimate answer, unlike the
    /// named-error identity lookups above. Several generated declarations
    /// may share one application anchor (every method of one `extend`
    /// block), so the result is a list.
    pub fn symbols_at(&self, file_id: u16, offset: usize) -> Vec<GeneratedSymbolProvenance<'_>> {
        let mut hits: Vec<GeneratedSymbolProvenance<'_>> = self
            .records
            .iter()
            .filter(|(_, record)| record.origin.source_anchor.contains(file_id, offset))
            .map(|(id, record)| self.provenance_view(*id, record))
            .collect();
        hits.sort_by(|a, b| a.decl_name.cmp(b.decl_name));
        hits
    }

    /// S5 query surface: the generated symbols answering to `name` — the
    /// full declaration name (`Point.answer`; generated free functions
    /// answer to their bare name) or, for qualified method names, the
    /// method segment after the receiver qualifier (`answer`). This is the
    /// name FILTER navigation consults (Decision 68 LSP behaviors 1/3):
    /// like [`Self::symbols_at`] it is a filter, not an identity lookup —
    /// an empty result means "no generated symbol answers to this name"
    /// and is a legitimate answer. Deterministic (declaration-name) order.
    pub fn symbols_named(&self, name: &str) -> Vec<GeneratedSymbolProvenance<'_>> {
        let mut hits: Vec<GeneratedSymbolProvenance<'_>> = self
            .records
            .iter()
            .filter(|(_, record)| {
                record.decl_name == name
                    || record
                        .decl_name
                        .rsplit_once('.')
                        .is_some_and(|(_, method)| method == name)
            })
            .map(|(id, record)| self.provenance_view(*id, record))
            .collect();
        hits.sort_by(|a, b| a.decl_name.cmp(b.decl_name));
        hits
    }

    /// Derived name view: resolve a generated declaration's function-table
    /// name to its issued SymbolId. An unknown name is a named error.
    pub fn symbol_for_name(&self, decl_name: &str) -> Result<SymbolId, String> {
        self.names
            .get(decl_name)
            .copied()
            .ok_or_else(|| UNKNOWN_GENERATED_SYMBOL_DIAGNOSTIC.to_string())
    }

    /// Derived name view: was this function-table name issued for a
    /// generated declaration? (The former `materialized_comptime_fns`
    /// membership check.)
    pub(crate) fn contains_name(&self, decl_name: &str) -> bool {
        self.names.contains_key(decl_name)
    }

    /// Number of reserved generated declarations.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Is the table empty? (Clippy pairing for [`Self::len`]; an empty
    /// table is the legitimate "no comptime expansions in this unit"
    /// state.)
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
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
            source_anchor: SourceAnchor::new(0, Span::new(42, 57)).expect("real span must anchor"),
        }
    }

    fn sample_content() -> CanonicalHash {
        CanonicalHash::from_canonical_decl_encoding("extend:Point:method:sum:body-encoding")
    }

    fn sample_generator_anchor() -> SourceAnchor {
        SourceAnchor::new(0, Span::new(5, 30)).expect("real generator span must anchor")
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
        let reservation_a = table
            .reserve_generated_decl(
                "Point.sum",
                sample_origin(),
                sample_content(),
                sample_generator_anchor(),
            )
            .expect("first reservation succeeds");
        let SymbolReservation::Fresh(id_a) = reservation_a else {
            panic!("first reservation must be Fresh, got {reservation_a:?}");
        };
        // Reserving the SAME identity + origin + content again (speculative
        // pre-pass then authoritative pass-2) is idempotent: same SymbolId,
        // reported as a re-issue, no conflict.
        let reservation_b = table
            .reserve_generated_decl(
                "Point.sum",
                sample_origin(),
                sample_content(),
                sample_generator_anchor(),
            )
            .expect("idempotent re-reservation succeeds");
        let SymbolReservation::Reissued(id_b) = reservation_b else {
            panic!("second reservation must be Reissued, got {reservation_b:?}");
        };
        assert_eq!(id_a, id_b);
        assert_eq!(table.len(), 1, "one identity, one record");
        assert_eq!(
            table.symbol_for_name("Point.sum").expect("name resolves"),
            id_a,
            "the derived name view resolves to the issued SymbolId"
        );
        assert!(table.contains_name("Point.sum"));
    }

    // Rejection row 3 at the table (the enforcement point the registration
    // path calls): same identity, conflicting content -> named error.
    #[test]
    fn conflicting_content_for_one_reserved_identity_is_the_named_row3_error() {
        let mut table = GeneratedSymbolTable::new();
        table
            .reserve_generated_decl(
                "Point.sum",
                sample_origin(),
                sample_content(),
                sample_generator_anchor(),
            )
            .expect("first reservation succeeds");
        let err = table
            .reserve_generated_decl(
                "Point.sum",
                sample_origin(),
                CanonicalHash::from_canonical_decl_encoding("a-different-body-encoding"),
                sample_generator_anchor(),
            )
            .expect_err("conflicting output for one identity must be refused");
        assert!(
            err.contains(GENERATED_SYMBOL_DUPLICATE_IDENTITY_DIAGNOSTIC),
            "row-3 named diagnostic missing: {err}"
        );
        assert!(
            err.contains("annotation:json_schema@app.main"),
            "row-3 error must carry generator provenance: {err}"
        );
    }

    // Rejection row 2 at the table: one declaration NAME reserved under two
    // different expansion identities -> named conflict error carrying both
    // expansions' provenance.
    #[test]
    fn one_name_under_two_identities_is_the_named_row2_error() {
        let mut table = GeneratedSymbolTable::new();
        table
            .reserve_generated_decl(
                "Point.sum",
                sample_origin(),
                sample_content(),
                sample_generator_anchor(),
            )
            .expect("first reservation succeeds");
        let other_origin = GeneratedOrigin {
            expansion: ExpansionIdentity::new(
                GeneratorRef::from_canonical_descriptor("annotation:json_schema@app.main"),
                ApplicationId::from_canonical_descriptor("application:app.main:Vec2:json_schema"),
                TargetIdentity::from_canonical_descriptor("type:app.main:Vec2"),
                ComptimeStage::AnnotationHandler,
                sample_arguments_hash(),
                sample_dependencies_hash(),
            ),
            node_path: GeneratedNodePath::decl_root("extend:Point").child("method:sum"),
            source_anchor: SourceAnchor::new(0, Span::new(90, 110)).expect("real span must anchor"),
        };
        let err = table
            .reserve_generated_decl(
                "Point.sum",
                other_origin,
                sample_content(),
                sample_generator_anchor(),
            )
            .expect_err("one name under two identities must be refused");
        assert!(
            err.contains(GENERATED_SYMBOL_CONFLICT_DIAGNOSTIC),
            "row-2 named diagnostic missing: {err}"
        );
        assert!(
            err.contains("type:app.main:Point") && err.contains("type:app.main:Vec2"),
            "row-2 error must carry BOTH expansions' provenance: {err}"
        );
    }

    // Unknown-identity lookups are named errors, never silent absences.
    #[test]
    fn unknown_symbol_lookups_are_named_errors() {
        let table = GeneratedSymbolTable::new();
        let err = table
            .symbol_for_name("never_generated")
            .expect_err("unknown name must be a named error");
        assert!(err.contains(UNKNOWN_GENERATED_SYMBOL_DIAGNOSTIC));
    }

    // S4 query surface: a reserved generated symbol resolves (by name and
    // by SymbolId) to its FULL provenance — {symbol, checked-decl location,
    // application location, generator-definition location} — answered from
    // the identity table alone. The checked-decl and application anchors
    // coincide until D2 virtual documents (documented field split); the
    // generator anchor is the handler-definition location.
    #[test]
    fn s4_query_surface_resolves_full_provenance_by_name_and_id() {
        let mut table = GeneratedSymbolTable::new();
        table
            .reserve_generated_decl(
                "Point.sum",
                sample_origin(),
                sample_content(),
                sample_generator_anchor(),
            )
            .expect("reservation succeeds");

        let by_name = table
            .provenance_for_name("Point.sum")
            .expect("reserved name resolves to provenance");
        assert_eq!(by_name.decl_name, "Point.sum");
        assert_eq!(by_name.checked_decl, sample_origin().source_anchor);
        assert_eq!(by_name.application, sample_origin().source_anchor);
        assert_eq!(by_name.generator, sample_generator_anchor());
        assert_eq!(
            by_name.node_path.render(),
            "extend:Point/method:sum",
            "node path renders root/child"
        );

        let by_id = table
            .provenance_of(by_name.symbol)
            .expect("issued SymbolId resolves to provenance");
        assert_eq!(by_id, by_name, "name view and id view agree");
    }

    // S4 query surface: unknown-SymbolId provenance lookups are the named
    // error — never a silent absent-value return. (The id is issued by a
    // DIFFERENT table; SymbolId's constructor stays private, so this is
    // the only way to hold an id the queried table never issued.)
    #[test]
    fn s4_unknown_symbol_id_provenance_lookup_is_the_named_error() {
        let mut issuing = GeneratedSymbolTable::new();
        let SymbolReservation::Fresh(foreign_id) = issuing
            .reserve_generated_decl(
                "Point.sum",
                sample_origin(),
                sample_content(),
                sample_generator_anchor(),
            )
            .expect("reservation succeeds")
        else {
            panic!("first reservation must be Fresh");
        };
        let empty = GeneratedSymbolTable::new();
        let err = empty
            .provenance_of(foreign_id)
            .expect_err("unknown SymbolId must be a named error");
        assert!(err.contains(UNKNOWN_GENERATED_SYMBOL_DIAGNOSTIC));
    }

    // S4 query surface: the workspace-symbol listing enumerates every
    // generated symbol with provenance in deterministic declaration-name
    // order, and the position filter resolves a byte offset inside a
    // checked-decl anchor to the declarations anchored there (a filter,
    // not an identity lookup: empty result = nothing generated here).
    #[test]
    fn s4_query_surface_lists_and_position_filters_generated_symbols() {
        let mut table = GeneratedSymbolTable::new();
        table
            .reserve_generated_decl(
                "Point.sum",
                sample_origin(),
                sample_content(),
                sample_generator_anchor(),
            )
            .expect("reserve sum");
        table
            .reserve_generated_decl(
                "Point.scale",
                GeneratedOrigin {
                    expansion: sample_identity(),
                    node_path: GeneratedNodePath::decl_root("extend:Point").child("method:scale"),
                    source_anchor: SourceAnchor::new(0, Span::new(42, 57))
                        .expect("real span must anchor"),
                },
                CanonicalHash::from_canonical_decl_encoding("scale-body-encoding"),
                sample_generator_anchor(),
            )
            .expect("reserve scale");

        let listing = table.generated_symbols();
        assert_eq!(
            listing
                .iter()
                .map(|provenance| provenance.decl_name)
                .collect::<Vec<_>>(),
            vec!["Point.scale", "Point.sum"],
            "workspace-symbol listing is deterministic by declaration name"
        );

        // Both decls share the application anchor (span 42..57 in file 0).
        let at_application = table.symbols_at(0, 50);
        assert_eq!(
            at_application
                .iter()
                .map(|provenance| provenance.decl_name)
                .collect::<Vec<_>>(),
            vec!["Point.scale", "Point.sum"],
            "position inside the shared checked-decl anchor resolves both methods"
        );

        assert!(
            table.symbols_at(0, 200).is_empty(),
            "an offset outside every anchor resolves to no generated symbols"
        );
        assert!(
            table.symbols_at(9, 50).is_empty(),
            "a different file id resolves to no generated symbols"
        );
    }

    // S5 query surface: the name FILTER navigation consults — a generated
    // symbol answers to its full declaration name ("Point.sum") and, for
    // qualified method names, to the method segment after the receiver
    // qualifier ("sum"). Like `symbols_at`, this is a filter, not an
    // identity lookup: an empty result means "no generated symbol answers
    // to this name" and is legitimate. Results are deterministic by
    // declaration name.
    #[test]
    fn s5_symbols_named_answers_qualified_and_simple_names() {
        let mut table = GeneratedSymbolTable::new();
        table
            .reserve_generated_decl(
                "Point.sum",
                sample_origin(),
                sample_content(),
                sample_generator_anchor(),
            )
            .expect("reserve Point.sum");
        table
            .reserve_generated_decl(
                "Vec2.sum",
                GeneratedOrigin {
                    expansion: ExpansionIdentity::new(
                        sample_identity().generator,
                        ApplicationId::from_canonical_descriptor(
                            "application:app.main:Vec2:json_schema",
                        ),
                        TargetIdentity::from_canonical_descriptor("type:app.main:Vec2"),
                        ComptimeStage::AnnotationHandler,
                        sample_arguments_hash(),
                        sample_dependencies_hash(),
                    ),
                    node_path: GeneratedNodePath::decl_root("extend:Vec2").child("method:sum"),
                    source_anchor: SourceAnchor::new(0, Span::new(90, 110))
                        .expect("real span must anchor"),
                },
                CanonicalHash::from_canonical_decl_encoding("vec2-sum-body-encoding"),
                sample_generator_anchor(),
            )
            .expect("reserve Vec2.sum");
        table
            .reserve_generated_decl(
                "free_schema_fn",
                GeneratedOrigin {
                    expansion: ExpansionIdentity::new(
                        sample_identity().generator,
                        ApplicationId::from_canonical_descriptor(
                            "application:app.main:free:json_schema",
                        ),
                        TargetIdentity::from_canonical_descriptor("type:app.main:Point"),
                        ComptimeStage::AnnotationHandler,
                        sample_arguments_hash(),
                        sample_dependencies_hash(),
                    ),
                    node_path: GeneratedNodePath::decl_root("fn:free_schema_fn"),
                    source_anchor: SourceAnchor::new(0, Span::new(120, 140))
                        .expect("real span must anchor"),
                },
                CanonicalHash::from_canonical_decl_encoding("free-fn-body-encoding"),
                sample_generator_anchor(),
            )
            .expect("reserve free_schema_fn");

        // Simple method-segment name answers every generated method with
        // that segment, deterministically ordered.
        assert_eq!(
            table
                .symbols_named("sum")
                .iter()
                .map(|provenance| provenance.decl_name)
                .collect::<Vec<_>>(),
            vec!["Point.sum", "Vec2.sum"],
        );
        // Full qualified declaration name answers exactly that symbol.
        assert_eq!(
            table
                .symbols_named("Point.sum")
                .iter()
                .map(|provenance| provenance.decl_name)
                .collect::<Vec<_>>(),
            vec!["Point.sum"],
        );
        // Generated free functions answer to their declaration name.
        assert_eq!(
            table
                .symbols_named("free_schema_fn")
                .iter()
                .map(|provenance| provenance.decl_name)
                .collect::<Vec<_>>(),
            vec!["free_schema_fn"],
        );
        // A name no generated symbol answers to: empty filter result.
        assert!(table.symbols_named("never_generated").is_empty());
        // The receiver qualifier alone is NOT a symbol name.
        assert!(table.symbols_named("Point").is_empty());
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
        let SymbolReservation::Fresh(sum) = table
            .reserve_generated_decl(
                "Point.sum",
                sample_origin(),
                sample_content(),
                sample_generator_anchor(),
            )
            .expect("reserve sum")
        else {
            panic!("first reservation must be Fresh");
        };
        let SymbolReservation::Fresh(scale) = table
            .reserve_generated_decl(
                "Point.scale",
                GeneratedOrigin {
                    expansion: sample_identity(),
                    node_path: GeneratedNodePath::decl_root("extend:Point").child("method:scale"),
                    source_anchor: SourceAnchor::new(0, Span::new(42, 57))
                        .expect("real span must anchor"),
                },
                sample_content(),
                sample_generator_anchor(),
            )
            .expect("reserve scale")
        else {
            panic!("distinct-name reservation must be Fresh");
        };
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

        let arg_split_one = CanonicalHash::from_canonical_argument_descriptors(&[("a", "bc")]);
        let arg_split_two = CanonicalHash::from_canonical_argument_descriptors(&[("ab", "c")]);
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
