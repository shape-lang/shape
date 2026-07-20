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
pub use shape_ast::ast::GeneratedNodePath;
use shape_ast::ast::Span;
use std::collections::HashMap;

mod node_origin;
#[cfg(test)]
mod test_support;

#[cfg(test)]
fn new_test_node_issuer() -> shape_ast::ast::GeneratedNodeIssuer {
    shape_ast::ast::GeneratedNodeIssuer::new()
}

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

// ─────────────────────────────────────────────────────────────────────────
// ADR-009 ticket D2 (Decision 67) — declaration-discovery fixed point.
//
// The monotonic declaration-discovery fixed point ([`DeclarationDiscoveryFixedPoint`])
// drives the existing run-once/dedup chokepoint ([`GeneratedSymbolTable::
// reserve_generated_decl`]) to a deterministic, monotone convergence BEFORE
// ordinary body checking. The five named diagnostics below are the D2
// rejection matrix; each is routed through the C0003 generated-declaration
// diagnostic family (`comptime_diagnostics.rs`) carrying expansion provenance.
// ─────────────────────────────────────────────────────────────────────────

/// Rejection-matrix D2 row (cycle): application A's generated declaration
/// re-triggers an application whose generated declaration re-triggers A — a
/// non-terminating output-triggers chain. A cycle in the discovery graph is
/// a compile error carrying the cyclic expansions' provenance chain
/// (Decision 67: cycles are compile errors).
pub(crate) const DISCOVERY_CYCLE_DIAGNOSTIC: &str = "declaration-discovery cycle: a generated declaration re-triggers an \
     expansion that (transitively) re-triggers it, so discovery never \
     reaches a fixed point (ADR-009 Decision 67)";

/// Rejection-matrix D2 row (oscillation): the discovered-header state
/// returns to an earlier non-terminal state instead of growing monotonically
/// to convergence (a header appears, then a later round's state flip-flops
/// back). Non-monotone discovery is a compile error (Decision 67:
/// discovery is monotone and converges).
pub(crate) const DISCOVERY_OSCILLATION_DIAGNOSTIC: &str = "declaration-discovery did not converge monotonically: the discovered \
     declaration set returned to an earlier state instead of growing to a \
     fixed point (ADR-009 Decision 67)";

/// Rejection-matrix D2 row (unbounded): the expansion count or the discovery
/// round depth exceeded the deterministic bound. Unbounded generation is a
/// compile error carrying the generator provenance (Decision 67:
/// unbounded generation is rejected).
pub(crate) const DISCOVERY_UNBOUNDED_DIAGNOSTIC: &str = "declaration-discovery exceeded the deterministic expansion bound: a \
     generator produced declarations without reaching a fixed point \
     (ADR-009 Decision 67)";

/// Rejection-matrix D2 row (header mutated): a declaration header discovered
/// in an earlier round is re-derived with DIFFERENT content later in the
/// same fixed point — a discovered header must be immutable through
/// discovery (Decision 67: discovered headers are immutable).
pub(crate) const DISCOVERY_HEADER_MUTATED_DIAGNOSTIC: &str = "declaration-discovery header mutated: a generated declaration that was \
     already discovered was re-derived with different content, violating \
     header immutability through the fixed point (ADR-009 Decision 67)";

/// Rejection-matrix D2 row (reserved-but-undefined): a generated-declaration
/// identity was reserved during discovery but never defined by the time the
/// fixed point converged. Every reserved identity must be defined exactly
/// once (Decision 67).
pub(crate) const RESERVED_IDENTITY_UNDEFINED_DIAGNOSTIC: &str = "reserved generated identity never defined: discovery reserved a \
     declaration header that was not defined before the fixed point \
     converged (ADR-009 Decision 67)";

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

// ─────────────────────────────────────────────────────────────────────────
// ADR-009 ticket E3 (slice S1, legacy class U10) — hygienic generated LOCALS.
//
// The compiler formerly named its generated locals / parameters / module
// bindings with synthetic, user-GUESSABLE spellings (`argN`, `__ann_args`,
// `__target_arg__`, …). A user local/argument spelled identically captured or
// shadowed the generated slot. Per Decision 68 + the E3 rejection matrix, a
// generated symbol's ROLE is bound by the compiler-issued token, never by a
// spelling: [`HygienicSymbol`] is the token, and its [`HygienicSymbol::
// unspellable_descriptor`] rendering (SOH-prefixed, the established reserved-
// name convention — `trait_evidence.rs`) can never be spelled in a Shape
// identifier, so a user reference to the former spelling resolves to a
// DIFFERENT string (unresolved), never the generated slot.
// ─────────────────────────────────────────────────────────────────────────

const HYGIENIC_SYMBOL_DOMAIN: &str = "adr009:u10:hygienic-symbol";

/// The ROLE a hygienic generated local / parameter / module binding plays,
/// bound by POSITION/TYPE — never by a user-guessable spelling. Each variant
/// replaces one former synthetic spelling; the discriminant (plus the
/// forwarder-param index) is the ONLY input to the identity besides the
/// compiler-issued nonce, so roles can never be confused and the same role in
/// two applications is disambiguated by the nonce.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HygienicRole {
    /// A comptime builtin-forwarder positional parameter (former `arg{N}`),
    /// disambiguated by its index so a forwarder's params never collide.
    ComptimeForwarderParam(u32),
    /// The comptime handler's target module binding (former `__target_arg__`).
    ComptimeTargetBinding,
    /// The comptime handler's compile-context module binding (former
    /// `__ctx_arg__`).
    ComptimeCtxBinding,
    /// The annotation before/after args local (former `__ann_args`).
    AnnotationArgs,
    /// The annotation before/after ctx local (former `__ann_ctx`).
    AnnotationCtx,
    /// The annotation-on-await subject local (former `__ann_subject`).
    AnnotationSubject,
    /// The annotation before/after result local (former `__ann_result`).
    AnnotationResult,
    /// The annotation before-handler result local (former
    /// `__ann_before_result`).
    AnnotationBeforeResult,
    // ── slice S2 (U10) — hygienic generated FUNCTION / HANDLER / WRAPPER
    // registry names. Each variant replaces one former synthetic function
    // spelling that entered a function table by name; the wrapper's ROLE is
    // now bound by the compiler-issued token, and its unspellable rendering
    // keeps a user function of the former spelling from colliding with — or
    // being resolved to — the generated wrapper.
    /// The `comptime { }` mini-program entry wrapper (former
    /// `__comptime_block__`). Program-isolated: minted once per mini-program.
    ComptimeBlockWrapper,
    /// The annotation-`comptime`-handler mini-program entry wrapper (former
    /// `__comptime_handler_fn__`). Program-isolated: minted once per
    /// mini-program.
    ComptimeHandlerWrapper,
    /// A specialized annotation before/after runtime handler registered in the
    /// outer function table (former `__ann_{name}_{before|after}_wrapper_{n}`).
    /// Disambiguated by the compiler nonce so before/after and every
    /// application register distinct slots (a name collision would make
    /// `find_function` return the wrong index).
    SpecializedAnnotationHandler,
    /// The foreign-function annotation wrapper slot name (former
    /// `{name}___ann_wrapper`). Cosmetic slot label — referenced by index —
    /// but must not be a user-spellable function name.
    ForeignAnnotationWrapper,
    // ── slice S3 (U11) — hygienic generated FUNCTION identity for the
    // pre-annotation body a `replace body` directive shadows. Replaces the
    // deleted `__original__{fn}` shadow spelling + the
    // `function_aliases["__original__"]` name-encoded alias: the shadow's
    // registry name is now the compiler-issued unspellable token, reachable
    // ONLY through the typed `ctx.original` capability. The nonce is derived
    // from the target function's name so re-registration is idempotent (one
    // shadow per annotated function, `register_function` dedups by name).
    /// The `replace body` pre-annotation body shadow (former `__original__{fn}`).
    OriginalBodyShadow,
    // ── slice S4 (U11) — hygienic generated FUNCTION identities for the
    // before/after runtime-hook wrapper CHAIN. Each replaces one former
    // user-spellable `{name}___…` chain name that entered the function table by
    // a guessable spelling; the wrapper's ROLE is now bound by the
    // compiler-issued token, and its unspellable rendering keeps a user
    // function of the former spelling (`compute___impl`, `compute___b`) from
    // colliding with — or being resolved to — the generated wrapper. The nonce
    // is a stable digest of the annotated function's name (+ the annotation
    // name for the intermediate wrapper) so re-registration is idempotent.
    /// The before/after wrapped original body (former `{name}___impl`).
    AnnotationHookImplBody,
    /// An intermediate before/after chain wrapper (former `{name}___{annotation}`).
    AnnotationHookWrapper,
    // ── ADR-009 C3 #14 (slice 2, S2c) — the typed hook-template weave's
    // hygienic impl identity: the target's FINAL body (post-directives,
    // post-`replace body`) moved under an unspellable shadow name so the
    // generated typed-AST wrapper compiled under the target's own name can
    // call it directly (the C3-G6 SMALL shape — bytecode AND MIR from the
    // same wrapped definition). The C3 successor of `AnnotationHookImplBody`
    // on the NEW path; the legacy role stays untouched beside it until the
    // S6 capstone deletes the legacy weave. The nonce is a stable digest of
    // the target function's name so re-registration is idempotent.
    /// The hook-template weave's impl body (the target's final body under
    /// the weave shadow).
    TemplateWeaveImplBody,
}

impl HygienicRole {
    /// Canonical descriptor digested into the hygienic identity — a stable
    /// role tag, never rendered source text.
    fn canonical_descriptor(&self) -> String {
        match self {
            Self::ComptimeForwarderParam(index) => {
                format!("role:comptime-forwarder-param:{index}")
            }
            Self::ComptimeTargetBinding => "role:comptime-target-binding".to_string(),
            Self::ComptimeCtxBinding => "role:comptime-ctx-binding".to_string(),
            Self::AnnotationArgs => "role:annotation-args".to_string(),
            Self::AnnotationCtx => "role:annotation-ctx".to_string(),
            Self::AnnotationSubject => "role:annotation-subject".to_string(),
            Self::AnnotationResult => "role:annotation-result".to_string(),
            Self::AnnotationBeforeResult => "role:annotation-before-result".to_string(),
            Self::ComptimeBlockWrapper => "role:comptime-block-wrapper".to_string(),
            Self::ComptimeHandlerWrapper => "role:comptime-handler-wrapper".to_string(),
            Self::SpecializedAnnotationHandler => "role:specialized-annotation-handler".to_string(),
            Self::ForeignAnnotationWrapper => "role:foreign-annotation-wrapper".to_string(),
            Self::OriginalBodyShadow => "role:original-body-shadow".to_string(),
            Self::AnnotationHookImplBody => "role:annotation-hook-impl-body".to_string(),
            Self::AnnotationHookWrapper => "role:annotation-hook-wrapper".to_string(),
            Self::TemplateWeaveImplBody => "role:template-weave-impl-body".to_string(),
        }
    }
}

/// Opaque compiler-issued identity of one generated local / parameter /
/// module binding (U10). Content-derived: 128 bits of SHA-256 over the role
/// descriptor plus the compiler-issued nonce — never a raw name string and
/// never a counter that participates in schema identity (the nonce
/// disambiguates within one compilation only, and never enters a symbol-table
/// key on its own; the identity does). The fields are private to this module
/// (the ProofGap pattern): emit code cannot fabricate a `HygienicSymbol`
/// without going through [`Self::mint`], which requires a role and a nonce.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HygienicSymbol {
    high: i64,
    low: i64,
}

impl HygienicSymbol {
    /// Mint a fresh hygienic identity for `role`, disambiguated by the
    /// compiler-issued `nonce`. Two mints with the SAME role+nonce agree (a
    /// parameter declaration and its reference within one generated wrapper);
    /// two mints with DIFFERENT nonces are distinct (two applications of one
    /// annotation mint distinct tokens).
    pub(crate) fn mint(role: HygienicRole, nonce: u64) -> Self {
        let nonce_descriptor = nonce.to_string();
        let hash = CanonicalHash::over_framed(
            HYGIENIC_SYMBOL_DOMAIN,
            [
                role.canonical_descriptor().as_str(),
                nonce_descriptor.as_str(),
            ],
        );
        Self {
            high: hash.high,
            low: hash.low,
        }
    }

    /// The UNSPELLABLE rendering used where a hygienic identity must flow
    /// through a by-name resolver — the comptime mini-program's parameter
    /// names / module bindings, and the scope name-table key of a hygienic
    /// local. SOH-prefixed (`\u{1}`), matching the established reserved-name
    /// convention (`trait_evidence.rs`), so it can never be spelled in a
    /// Shape identifier: a user reference to the former spelling
    /// (`__target_arg__`) is a DIFFERENT, resolvable-to-nothing string.
    pub(crate) fn unspellable_descriptor(&self) -> String {
        format!(
            "\u{1}hygienic:{:016x}{:016x}",
            self.high as u64, self.low as u64
        )
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

    /// ADR-009 C2 #13: undo a `Fresh` reservation on install rollback.
    ///
    /// The install transaction's undo journal (`compiler::checked_body`)
    /// records each `Fresh(id)` it made and replays this to remove EXACTLY
    /// those, restoring the pre-install table WITHOUT disturbing below-watermark
    /// reservations (dependency-module compiles, or earlier successful installs
    /// on a reused compiler — the H2 finding a wholesale reset destroyed). A
    /// `Fresh` reservation is genuinely new — a same-name reservation under a
    /// different identity errors instead of displacing — so removal is exact and
    /// never orphans a surviving reservation's name index.
    pub(crate) fn remove_reservation(&mut self, decl_name: &str, id: SymbolId) {
        self.records.remove(&id);
        if self.names.get(decl_name) == Some(&id) {
            self.names.remove(decl_name);
        }
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

/// Deterministic discovery bounds (ADR-009 Decision 67: unbounded generation
/// is a compile error). The bounds are generous relative to any realistic
/// program — they exist to make non-convergence a terminating compile error
/// rather than a hang. Exceeding EITHER bound is the named
/// [`DISCOVERY_UNBOUNDED_DIAGNOSTIC`].
const MAX_DISCOVERY_ROUNDS: u32 = 256;
const MAX_DISCOVERY_EXPANSIONS: u32 = 65_536;

/// Outcome of claiming one expansion application against the run-once memo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApplicationClaim {
    /// First time this application identity is seen this compilation unit:
    /// the driver must execute the handler and record its generated headers.
    Fresh,
    /// The run-once memo already holds this identity (same ApplicationId +
    /// dependencies hash): the handler ran once already — skip re-execution.
    /// This is the memoized short-circuit, NOT a silent failure skip.
    AlreadyApplied,
}

/// ADR-009 ticket D2 (Decision 67): the monotonic declaration-discovery
/// fixed point. It is the deterministic driver state threaded through the
/// existing speculative pre-pass
/// (`functions_annotations.rs::materialize_computed_comptime_extends`),
/// converting it from a single unbounded speculative pass into a bounded
/// worklist that reaches a fixed point BEFORE ordinary body checking.
///
/// The fixed point is the SINGLE discovery pass — there is no speculative
/// second evaluation; the compiler and the LSP consume the SAME reserved
/// [`GeneratedSymbolTable`] this fixed point drives (Decision 66). This type
/// owns only the convergence accounting; the identity/dedup chokepoint stays
/// [`GeneratedSymbolTable::reserve_generated_decl`].
///
/// Invariants enforced (each a named D2 diagnostic):
/// - run-once per application identity ([`ApplicationClaim`]);
/// - discovered headers immutable through discovery
///   ([`DISCOVERY_HEADER_MUTATED_DIAGNOSTIC`]);
/// - monotone convergence, no oscillation
///   ([`DISCOVERY_OSCILLATION_DIAGNOSTIC`]);
/// - no output-triggers cycles ([`DISCOVERY_CYCLE_DIAGNOSTIC`]);
/// - bounded rounds/expansions ([`DISCOVERY_UNBOUNDED_DIAGNOSTIC`]);
/// - every reserved identity defined at convergence
///   ([`RESERVED_IDENTITY_UNDEFINED_DIAGNOSTIC`]).
#[derive(Debug)]
pub(crate) struct DeclarationDiscoveryFixedPoint {
    /// Rounds begun so far (each worklist drain is one round).
    rounds: u32,
    /// Applications claimed so far (the expansion-count bound axis).
    expansions: u32,
    /// Run-once memo: application-identity fingerprints already claimed.
    claimed: std::collections::HashSet<CanonicalHash>,
    /// Discovered-header immutability ledger: declaration name → the content
    /// fingerprint it was first discovered with. Monotone: names are only
    /// added; re-derivation with different content is the mutation error.
    header_content: HashMap<String, CanonicalHash>,
    /// Reserved-vs-defined accounting. `reserved` names must all appear in
    /// `defined` at [`Self::converge`].
    reserved: std::collections::BTreeSet<String>,
    defined: std::collections::BTreeSet<String>,
    /// Output-triggers adjacency (application fingerprint → applications its
    /// generated declarations trigger). Used for cycle detection.
    triggers: HashMap<CanonicalHash, Vec<CanonicalHash>>,
    /// State fingerprints observed at each round boundary, oldest first. A
    /// non-adjacent repeat is oscillation; an adjacent repeat is convergence.
    round_states: Vec<CanonicalHash>,
}

impl DeclarationDiscoveryFixedPoint {
    const STATE_DOMAIN: &'static str = "adr009:d2:discovery-state";

    /// One fixed point per compilation unit, threaded through the pre-pass.
    pub(crate) fn new() -> Self {
        Self {
            rounds: 0,
            expansions: 0,
            claimed: std::collections::HashSet::new(),
            header_content: HashMap::new(),
            reserved: std::collections::BTreeSet::new(),
            defined: std::collections::BTreeSet::new(),
            triggers: HashMap::new(),
            round_states: Vec::new(),
        }
    }

    /// Begin one discovery round (one worklist drain). Bounds the round depth
    /// ([`DISCOVERY_UNBOUNDED_DIAGNOSTIC`]).
    pub(crate) fn begin_round(&mut self) -> Result<(), String> {
        self.rounds += 1;
        if self.rounds > MAX_DISCOVERY_ROUNDS {
            return Err(format!(
                "{DISCOVERY_UNBOUNDED_DIAGNOSTIC}: exceeded {MAX_DISCOVERY_ROUNDS} discovery rounds"
            ));
        }
        Ok(())
    }

    /// Claim one expansion application against the run-once memo. Bounds the
    /// expansion count ([`DISCOVERY_UNBOUNDED_DIAGNOSTIC`]); a fingerprint
    /// already claimed returns [`ApplicationClaim::AlreadyApplied`] so the
    /// driver skips re-execution (run-once per ApplicationId + deps hash).
    pub(crate) fn claim(
        &mut self,
        identity: &ExpansionIdentity,
    ) -> Result<ApplicationClaim, String> {
        let fingerprint = identity.fingerprint();
        if self.claimed.contains(&fingerprint) {
            return Ok(ApplicationClaim::AlreadyApplied);
        }
        self.expansions += 1;
        if self.expansions > MAX_DISCOVERY_EXPANSIONS {
            return Err(format!(
                "{DISCOVERY_UNBOUNDED_DIAGNOSTIC}: exceeded {MAX_DISCOVERY_EXPANSIONS} expansions"
            ));
        }
        self.claimed.insert(fingerprint);
        Ok(ApplicationClaim::Fresh)
    }

    /// Reserve a generated declaration header without (yet) defining it — the
    /// predeclare-then-define shape. A reserved-but-never-defined header is the
    /// [`RESERVED_IDENTITY_UNDEFINED_DIAGNOSTIC`] at convergence. The v1
    /// pre-pass reserves-and-defines atomically via [`Self::record_header`], so
    /// this split accessor drives the reserved-vs-defined convergence tests.
    #[cfg(test)]
    pub(crate) fn reserve_header(&mut self, decl_name: &str) {
        self.reserved.insert(decl_name.to_string());
    }

    /// Record a discovered header with its content fingerprint: reserves AND
    /// defines it, and enforces immutability. Re-recording the same name with
    /// a DIFFERENT content fingerprint is [`DISCOVERY_HEADER_MUTATED_DIAGNOSTIC`];
    /// re-recording with the SAME content is idempotent (the run-once memo
    /// normally prevents re-derivation, but a re-issued reservation is benign).
    pub(crate) fn record_header(
        &mut self,
        decl_name: &str,
        content: CanonicalHash,
    ) -> Result<(), String> {
        if let Some(existing) = self.header_content.get(decl_name) {
            if *existing != content {
                return Err(format!(
                    "{DISCOVERY_HEADER_MUTATED_DIAGNOSTIC}: `{decl_name}` was discovered with one \
                     definition and re-derived with a different one"
                ));
            }
        }
        self.header_content.insert(decl_name.to_string(), content);
        self.reserved.insert(decl_name.to_string());
        self.defined.insert(decl_name.to_string());
        Ok(())
    }

    /// Mark a previously reserved header as defined (pairs with
    /// [`Self::reserve_header`] for the predeclare-then-define shape;
    /// convergence-test accessor).
    #[cfg(test)]
    pub(crate) fn define_header(
        &mut self,
        decl_name: &str,
        content: CanonicalHash,
    ) -> Result<(), String> {
        self.record_header(decl_name, content)
    }

    /// Record an output-triggers edge: application `from`'s generated
    /// declaration re-triggered application `to`. Adding an edge that closes
    /// a cycle in the discovery graph is [`DISCOVERY_CYCLE_DIAGNOSTIC`].
    pub(crate) fn record_trigger(
        &mut self,
        from: &ExpansionIdentity,
        to: &ExpansionIdentity,
    ) -> Result<(), String> {
        let from_fp = from.fingerprint();
        let to_fp = to.fingerprint();
        // A self-edge is a trivial cycle.
        if from_fp == to_fp || self.reaches(to_fp, from_fp) {
            return Err(format!(
                "{DISCOVERY_CYCLE_DIAGNOSTIC}: `{}` → `{}` closes a cycle",
                from.application.canonical_descriptor(),
                to.application.canonical_descriptor(),
            ));
        }
        self.triggers.entry(from_fp).or_default().push(to_fp);
        Ok(())
    }

    /// Is `target` reachable from `start` along trigger edges?
    fn reaches(&self, start: CanonicalHash, target: CanonicalHash) -> bool {
        let mut stack = vec![start];
        let mut seen = std::collections::HashSet::new();
        while let Some(node) = stack.pop() {
            if node == target {
                return true;
            }
            if !seen.insert(node) {
                continue;
            }
            if let Some(next) = self.triggers.get(&node) {
                stack.extend(next.iter().copied());
            }
        }
        false
    }

    /// Observe the discovery STATE at a round boundary. `frontier` is the
    /// driver's canonical descriptor of the round's frontier (the sorted
    /// pending-worklist type identities plus the discovered-header names): a
    /// correct monotone run drives the frontier toward emptiness and never
    /// revisits a prior configuration. A state that repeats a NON-adjacent
    /// earlier state means discovery returned to a prior configuration
    /// instead of converging — [`DISCOVERY_OSCILLATION_DIAGNOSTIC`]. An
    /// adjacent repeat (unchanged from the previous round) is the benign
    /// convergence signal, not an error.
    pub(crate) fn observe_round_state(&mut self, frontier: &[String]) -> Result<(), String> {
        let state = self.frontier_fingerprint(frontier);
        if let Some(last) = self.round_states.last() {
            if *last == state {
                // Unchanged since the previous round: converged, not an error.
                self.round_states.push(state);
                return Ok(());
            }
        }
        if self.round_states.contains(&state) {
            return Err(format!(
                "{DISCOVERY_OSCILLATION_DIAGNOSTIC}: the discovery frontier recurred after \
                 changing, so discovery is not monotone"
            ));
        }
        self.round_states.push(state);
        Ok(())
    }

    /// Fingerprint over the sorted round-frontier descriptor — stable,
    /// insensitive to the order the driver lists the frontier in.
    fn frontier_fingerprint(&self, frontier: &[String]) -> CanonicalHash {
        let mut parts: Vec<&str> = frontier.iter().map(String::as_str).collect();
        parts.sort_unstable();
        CanonicalHash::over_framed(Self::STATE_DOMAIN, parts)
    }

    /// The fixed point is reached: every reserved identity must be defined
    /// ([`RESERVED_IDENTITY_UNDEFINED_DIAGNOSTIC`]).
    pub(crate) fn converge(&self) -> Result<(), String> {
        let undefined: Vec<&String> = self.reserved.difference(&self.defined).collect();
        if let Some(first) = undefined.first() {
            return Err(format!(
                "{RESERVED_IDENTITY_UNDEFINED_DIAGNOSTIC}: `{first}` was reserved but never defined"
            ));
        }
        Ok(())
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

    // U10 (slice S1): the hygienic-symbol mint is deterministic for one
    // role+nonce, distinct across nonces (two applications of one annotation
    // mint distinct tokens), distinct across roles, and renders an
    // UNSPELLABLE descriptor (SOH-prefixed) that can never be an ordinary
    // Shape identifier — so a user reference to the former synthetic spelling
    // resolves to a DIFFERENT string, never the generated slot.
    #[test]
    fn hygienic_symbol_mint_is_deterministic_distinct_and_unspellable() {
        // Determinism: same role + same nonce agree (a parameter declaration
        // and its reference within one generated wrapper must render equal).
        let a = HygienicSymbol::mint(HygienicRole::AnnotationArgs, 7);
        let b = HygienicSymbol::mint(HygienicRole::AnnotationArgs, 7);
        assert_eq!(a, b, "same role+nonce mints one identity");
        assert_eq!(a.unspellable_descriptor(), b.unspellable_descriptor());

        // Distinct across nonces: two applications of one annotation.
        let c = HygienicSymbol::mint(HygienicRole::AnnotationArgs, 8);
        assert_ne!(a, c, "distinct nonces mint distinct tokens");
        assert_ne!(a.unspellable_descriptor(), c.unspellable_descriptor());

        // Distinct across roles at one nonce (role bound by position/type).
        let ctx = HygienicSymbol::mint(HygienicRole::AnnotationCtx, 7);
        assert_ne!(a, ctx, "distinct roles mint distinct tokens");

        // Distinct across forwarder-param indices.
        let p0 = HygienicSymbol::mint(HygienicRole::ComptimeForwarderParam(0), 0);
        let p1 = HygienicSymbol::mint(HygienicRole::ComptimeForwarderParam(1), 0);
        assert_ne!(p0, p1, "forwarder params are indexed distinctly");

        // Unspellable: the SOH (\u{1}) prefix is rejected by the identifier
        // grammar, so the descriptor is not a user-addressable spelling. The
        // former synthetic spelling is a plain identifier and can never equal
        // this rendering.
        let desc = a.unspellable_descriptor();
        assert!(
            desc.starts_with('\u{1}'),
            "descriptor must be SOH-prefixed: {desc:?}"
        );
        assert_ne!(
            desc, "__ann_args",
            "unspellable descriptor never equals the former spelling"
        );
        assert!(
            !desc.chars().next().unwrap().is_alphabetic(),
            "descriptor cannot start like a Shape identifier"
        );
    }

    // U10 (slice S2): the four hygienic FUNCTION / HANDLER / WRAPPER roles
    // mint distinct, unspellable identities. Each replaces one former
    // synthetic function spelling that entered a function table by name; the
    // unspellable rendering guarantees a user function of the former spelling
    // can neither collide with nor be resolved to the generated wrapper.
    #[test]
    fn hygienic_function_roles_mint_distinct_unspellable_identities() {
        let block = HygienicSymbol::mint(HygienicRole::ComptimeBlockWrapper, 0);
        let handler = HygienicSymbol::mint(HygienicRole::ComptimeHandlerWrapper, 0);
        let specialized = HygienicSymbol::mint(HygienicRole::SpecializedAnnotationHandler, 0);
        let foreign = HygienicSymbol::mint(HygienicRole::ForeignAnnotationWrapper, 0);

        // The four roles are pairwise distinct at one nonce (role bound by
        // position/type, not spelling).
        let ids = [block, handler, specialized, foreign];
        for (i, a) in ids.iter().enumerate() {
            for b in ids.iter().skip(i + 1) {
                assert_ne!(a, b, "distinct function roles mint distinct tokens");
            }
        }

        // Program-isolated wrappers mint deterministically at a fixed nonce
        // (one wrapper per mini-program; reproducible output).
        assert_eq!(
            block,
            HygienicSymbol::mint(HygienicRole::ComptimeBlockWrapper, 0),
            "same role+nonce agree — decl and reference of the mini-program \
             wrapper must render equal"
        );

        // Outer-registry wrappers are disambiguated by the compiler nonce so
        // before/after and every application register distinct slots.
        let specialized_next = HygienicSymbol::mint(HygienicRole::SpecializedAnnotationHandler, 1);
        assert_ne!(
            specialized, specialized_next,
            "distinct nonces mint distinct specialized-handler slots"
        );

        // Every rendering is UNSPELLABLE (SOH-prefixed) and never equals the
        // former synthetic spelling — rejection row 2 (no magic spelling in a
        // symbol table) proven at the identity layer.
        for (id, former) in [
            (block, "__comptime_block__"),
            (handler, "__comptime_handler_fn__"),
            (specialized, "__ann_logged_before_wrapper_0"),
            (foreign, "work___ann_wrapper"),
        ] {
            let desc = id.unspellable_descriptor();
            assert!(
                desc.starts_with('\u{1}'),
                "descriptor must be SOH-prefixed: {desc:?}"
            );
            assert_ne!(
                desc, former,
                "unspellable descriptor never equals the former spelling"
            );
        }
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

    // ── ADR-009 D2 declaration-discovery fixed-point ledger tests ──

    fn identity_named(application: &str, deps: &[&str]) -> ExpansionIdentity {
        ExpansionIdentity::new(
            GeneratorRef::from_canonical_descriptor("annotation:derive@app.main"),
            ApplicationId::from_canonical_descriptor(application),
            TargetIdentity::from_canonical_descriptor("type:app.main:Point"),
            ComptimeStage::AnnotationHandler,
            sample_arguments_hash(),
            CanonicalHash::from_canonical_dependency_descriptors(deps),
        )
    }

    // Convergence: a monotone run claims each application once, records its
    // headers, and converges cleanly. A second claim of the same identity is
    // memoized (AlreadyApplied) — the run-once guarantee.
    #[test]
    fn discovery_reaches_monotone_fixed_point_running_each_application_once() {
        let mut fp = DeclarationDiscoveryFixedPoint::new();
        let a = identity_named(
            "application:app.main:Point:derive",
            &["type:app.main:Point"],
        );

        fp.begin_round().expect("round 1 within bound");
        assert_eq!(
            fp.claim(&a).expect("first claim ok"),
            ApplicationClaim::Fresh,
            "first sighting of an application identity is Fresh"
        );
        fp.record_header("Point.derived", sample_content())
            .expect("record header");
        // Re-claiming the SAME identity (e.g. the struct re-enqueued) is the
        // run-once short-circuit, not a re-execution.
        assert_eq!(
            fp.claim(&a).expect("second claim ok"),
            ApplicationClaim::AlreadyApplied,
            "the run-once memo short-circuits a re-claim"
        );

        // A second round observing an empty frontier converges.
        fp.observe_round_state(&["Point.derived".to_string()])
            .expect("round-1 frontier state");
        fp.begin_round().expect("round 2 within bound");
        fp.observe_round_state(&[])
            .expect("empty frontier converges");
        fp.converge().expect("every reserved header defined");
    }

    // Run-once is keyed on the FULL application identity: two applications
    // that differ ONLY in the dependencies hash are distinct (both Fresh),
    // while the identical identity is memoized.
    #[test]
    fn discovery_run_once_is_keyed_on_application_id_plus_dependencies_hash() {
        let mut fp = DeclarationDiscoveryFixedPoint::new();
        let deps_a = identity_named("application:app.main:Point:derive", &["type:app.main:A"]);
        let deps_b = identity_named("application:app.main:Point:derive", &["type:app.main:B"]);
        fp.begin_round().expect("round");
        assert_eq!(fp.claim(&deps_a).expect("claim a"), ApplicationClaim::Fresh);
        assert_eq!(
            fp.claim(&deps_b).expect("claim b"),
            ApplicationClaim::Fresh,
            "a different dependencies hash is a different application identity"
        );
        assert_eq!(
            fp.claim(&deps_a).expect("re-claim a"),
            ApplicationClaim::AlreadyApplied,
        );
    }

    // Immutability: a discovered header re-derived with different content is
    // the named DISCOVERY_HEADER_MUTATED error; re-recording identical content
    // is idempotent.
    #[test]
    fn discovery_header_is_immutable_through_the_fixed_point() {
        let mut fp = DeclarationDiscoveryFixedPoint::new();
        fp.record_header("Point.derived", sample_content())
            .expect("first record");
        fp.record_header("Point.derived", sample_content())
            .expect("identical re-record is idempotent");
        let err = fp
            .record_header(
                "Point.derived",
                CanonicalHash::from_canonical_decl_encoding("a-different-body"),
            )
            .expect_err("re-derivation with different content must be refused");
        assert!(
            err.contains(DISCOVERY_HEADER_MUTATED_DIAGNOSTIC),
            "header-mutation diagnostic missing: {err}"
        );
    }

    // Cycle: A's output triggers B, B's output triggers A → named
    // DISCOVERY_CYCLE carrying the application provenance.
    #[test]
    fn discovery_cycle_is_the_named_error() {
        let mut fp = DeclarationDiscoveryFixedPoint::new();
        let a = identity_named("application:app.main:A:derive", &["type:app.main:A"]);
        let b = identity_named("application:app.main:B:derive", &["type:app.main:B"]);
        fp.record_trigger(&a, &b).expect("A→B is acyclic");
        let err = fp
            .record_trigger(&b, &a)
            .expect_err("B→A closes a cycle and must be refused");
        assert!(
            err.contains(DISCOVERY_CYCLE_DIAGNOSTIC),
            "cycle diagnostic missing: {err}"
        );
        assert!(
            err.contains("application:app.main:B:derive"),
            "cycle error must carry the closing application's provenance: {err}"
        );
    }

    // A self-trigger is a trivial cycle.
    #[test]
    fn discovery_self_trigger_is_a_cycle() {
        let mut fp = DeclarationDiscoveryFixedPoint::new();
        let a = identity_named("application:app.main:A:derive", &["type:app.main:A"]);
        let err = fp
            .record_trigger(&a, &a)
            .expect_err("self-trigger is a trivial cycle");
        assert!(err.contains(DISCOVERY_CYCLE_DIAGNOSTIC));
    }

    // Oscillation: the discovery frontier returns to an earlier non-adjacent
    // state (A → B → A) → named DISCOVERY_OSCILLATION.
    #[test]
    fn discovery_oscillation_is_the_named_error() {
        let mut fp = DeclarationDiscoveryFixedPoint::new();
        fp.observe_round_state(&["A".to_string()]).expect("state A");
        fp.observe_round_state(&["B".to_string()]).expect("state B");
        let err = fp
            .observe_round_state(&["A".to_string()])
            .expect_err("returning to state A is oscillation");
        assert!(
            err.contains(DISCOVERY_OSCILLATION_DIAGNOSTIC),
            "oscillation diagnostic missing: {err}"
        );
    }

    // An unchanged frontier across adjacent rounds is convergence, not
    // oscillation.
    #[test]
    fn discovery_adjacent_repeat_is_convergence_not_oscillation() {
        let mut fp = DeclarationDiscoveryFixedPoint::new();
        fp.observe_round_state(&["A".to_string()]).expect("state A");
        fp.observe_round_state(&["A".to_string()])
            .expect("an unchanged frontier is the convergence signal");
    }

    // Unbounded: exceeding the deterministic round bound → DISCOVERY_UNBOUNDED.
    #[test]
    fn discovery_unbounded_rounds_is_the_named_error() {
        let mut fp = DeclarationDiscoveryFixedPoint::new();
        let mut err = None;
        for _ in 0..(MAX_DISCOVERY_ROUNDS + 8) {
            if let Err(e) = fp.begin_round() {
                err = Some(e);
                break;
            }
        }
        let err = err.expect("the round bound must eventually trip");
        assert!(
            err.contains(DISCOVERY_UNBOUNDED_DIAGNOSTIC),
            "unbounded diagnostic missing: {err}"
        );
    }

    // Unbounded: exceeding the deterministic expansion bound → DISCOVERY_UNBOUNDED.
    #[test]
    fn discovery_unbounded_expansions_is_the_named_error() {
        let mut fp = DeclarationDiscoveryFixedPoint::new();
        let mut err = None;
        for i in 0..(MAX_DISCOVERY_EXPANSIONS + 8) {
            let id = identity_named(
                &format!("application:app.main:gen{i}:derive"),
                &[Box::leak(format!("type:app.main:D{i}").into_boxed_str())],
            );
            if let Err(e) = fp.claim(&id) {
                err = Some(e);
                break;
            }
        }
        let err = err.expect("the expansion bound must eventually trip");
        assert!(
            err.contains(DISCOVERY_UNBOUNDED_DIAGNOSTIC),
            "unbounded diagnostic missing: {err}"
        );
    }

    // Reserved-but-undefined: a header reserved during discovery but never
    // defined is the named RESERVED_IDENTITY_UNDEFINED at convergence.
    #[test]
    fn discovery_reserved_but_undefined_identity_is_the_named_error() {
        let mut fp = DeclarationDiscoveryFixedPoint::new();
        fp.reserve_header("Point.pending");
        let err = fp
            .converge()
            .expect_err("a reserved-but-undefined header must fail convergence");
        assert!(
            err.contains(RESERVED_IDENTITY_UNDEFINED_DIAGNOSTIC),
            "reserved-undefined diagnostic missing: {err}"
        );
        // Defining it clears the failure.
        fp.define_header("Point.pending", sample_content())
            .expect("define");
        fp.converge().expect("now every reserved header is defined");
    }

    // ---- S6 close-out: repo-wide grep sentinels (rows 9/10), the
    // ---- `executor/tests/no_dynamic.rs` pattern. Documentation trees
    // ---- (docs/, CLAUDE.md) intentionally discuss these by name and are
    // ---- NOT scanned; source comments may NAME the D2 scope exclusion but
    // ---- code may not implement it. Needles are assembled from fragments
    // ---- so this file never contains them contiguously.

    /// Walk the repo's Rust source scope (crates/, bin/, tools/,
    /// extensions/) collecting each `.rs` file's contents.
    fn source_rs_contents() -> Vec<(std::path::PathBuf, String)> {
        // CARGO_MANIFEST_DIR = <repo>/crates/shape-vm — repo root is two up.
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest
            .parent()
            .and_then(std::path::Path::parent)
            .expect("repo root (two levels above crates/shape-vm)");
        let mut out = Vec::new();
        for dir in ["crates", "bin", "tools", "extensions"] {
            collect_rs(&repo_root.join(dir), &mut out);
        }
        assert!(
            !out.is_empty(),
            "sentinel scope must resolve source files from {repo_root:?}"
        );
        out
    }

    fn collect_rs(dir: &std::path::Path, out: &mut Vec<(std::path::PathBuf, String)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n == "target") {
                    continue;
                }
                collect_rs(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                if let Ok(src) = std::fs::read_to_string(&path) {
                    out.push((path, src));
                }
            }
        }
    }

    // Rejection row 9 (D2) — RELAXED + REPLACED (ADR-009 ticket D2).
    //
    // D1 reserved the declaration-discovery fixed-point / query-graph
    // vocabulary FOR ticket D2 by asserting its ABSENCE. D2 slice 1
    // IMPLEMENTS that surface, so the absence asserts are relaxed and
    // REPLACED (never deleted-and-forgotten) with POSITIVE assertions that
    //   (a) the fixed-point surface EXISTS in this module, and
    //   (b) the discovery driver is SINGLE-PASS — invoked exactly once at
    //       the compiler entry point, with no speculative second evaluation
    //       (Decision 66: the compiler and LSP consume the SAME reserved
    //       table this one pass drives).
    // The `shape-expansion://` virtual-document URI is IMPLEMENTED by D2
    // slice 3 (`tools/shape-lsp/src/expansion_views.rs`). D1 reserved the
    // vocabulary by asserting its ABSENCE; slice 1 unblocked it and slice 3
    // introduces it — so the absence assert is RELAXED + REPLACED (never
    // deleted-and-forgotten) with a POSITIVE assertion that the read-only
    // URI scheme now exists in CODE (a non-comment line), not just in scope
    // notes.
    #[test]
    fn row9_d2_declaration_discovery_surface_is_present_and_single_pass() {
        // (a-guard, slice 3) the virtual-document URI scheme is now
        // IMPLEMENTED — it must appear in a non-comment CODE line of the LSP
        // virtual-views surface.
        let uri_needle = ["shape-expa", "nsion://"].concat();
        let mut code_hits = Vec::new();
        for (path, src) in source_rs_contents() {
            for (line_index, line) in src.lines().enumerate() {
                if line.contains(&uri_needle) && !line.trim_start().starts_with("//") {
                    code_hits.push(format!("{}:{}", path.display(), line_index + 1));
                }
            }
        }
        assert!(
            !code_hits.is_empty(),
            "the D2 read-only virtual-document URI scheme must be implemented \
             in CODE (D2 slice 3, `expansion_views.rs`) — no non-comment code \
             line carries the `shape-expansion://` scheme"
        );

        // (a) POSITIVE: the declaration-discovery fixed-point surface exists
        // in this module — the vocabulary D1 reserved is now implemented.
        let this = include_str!("expansion_provenance.rs");
        let surface_needle = ["Declaration", "DiscoveryFixedPoint"].concat();
        assert!(
            this.contains(&surface_needle),
            "the D2 declaration-discovery fixed-point surface must exist \
             (relaxed row-9 replacement)"
        );

        // (b) SINGLE-PASS: the discovery driver is invoked exactly ONCE at
        // the compiler entry point — no speculative second pass.
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest
            .parent()
            .and_then(std::path::Path::parent)
            .expect("repo root");
        let driver_src = std::fs::read_to_string(
            repo_root.join("crates/shape-vm/src/compiler/compiler_impl_reference_model.rs"),
        )
        .expect("driver source resolves");
        let invocation = "materialize_computed_comptime_extends(&program)";
        let call_sites = driver_src.matches(invocation).count();
        assert_eq!(
            call_sites, 1,
            "the declaration-discovery driver must be invoked exactly once \
             (single-pass, no speculative second evaluation), found {call_sites}"
        );
    }

    // Rejection row 10 (forbidden carrier shapes): no provenance/expansion
    // bridge-shim-adapter renames (CLAUDE.md refuse-on-sight family), no
    // optional expansion identity ("stamp provenance later"), and no public
    // name-string SymbolId constructor anywhere in the identity module's
    // crate scope.
    #[test]
    fn row10_forbidden_provenance_carrier_shapes_are_absent() {
        let stems = ["provenance", "expansion", "generated_symbol", "symbol_id"];
        let suffixes = ["bridge", "shim", "adapter", "probe", "translator"];
        let optional_identity = ["Opt", "ion<", "Expansion", "Identity"].concat();
        let name_ctor = ["pub fn from_", "name("].concat();
        let mut hits = Vec::new();
        for (path, src) in source_rs_contents() {
            let lowered = src.to_lowercase();
            for stem in &stems {
                for suffix in &suffixes {
                    let snake = [stem, "_", suffix].concat();
                    let fused = [stem.replace('_', "").as_str(), suffix].concat();
                    if lowered.contains(&snake) || lowered.contains(&fused) {
                        hits.push(format!("{}: {stem}×{suffix}", path.display()));
                    }
                }
            }
            if src.contains(&optional_identity) {
                hits.push(format!("{}: optional expansion identity", path.display()));
            }
            if path.to_string_lossy().contains("comptime_builtins") && src.contains(&name_ctor) {
                hits.push(format!(
                    "{}: public name-string constructor in the identity scope",
                    path.display()
                ));
            }
        }
        assert!(
            hits.is_empty(),
            "forbidden provenance-carrier shape found (CLAUDE.md \
             refuse-on-sight family / row 10): {hits:#?}"
        );
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
