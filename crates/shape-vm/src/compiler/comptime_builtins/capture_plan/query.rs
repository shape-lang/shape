//! Read-only compiler query for validated generated capture descriptors.
//!
//! Capture identity comes only from the compiler-issued expansion origin and
//! resolved slot carried by the compiler's capture pack. Source spans are presentation
//! data: this module publishes them only when every span round-trips through
//! a structural AST node in the caller's source program. Post-U03-deletion the
//! source-string reparse route that could produce an unmapped generated capture
//! is gone, so a validated capture with no structural source map is an invariant
//! violation — quarantined as a [C0911] conflict, never published with an
//! invented source location.

use crate::compiler::{BytecodeCompiler, SourceAnchor};
use shape_ast::ast::{CaptureMode, Program};
use shape_runtime::type_system::GeneratedNodeKey;

mod aggregation;
mod identity;
mod source_index;
mod specialization;
use aggregation::CaptureAccumulator;
pub use identity::{
    GeneratedCaptureBindingIdentity, GeneratedCaptureOccurrenceIdentity, GeneratedCaptureSlot,
};
use source_index::AuthoredCaptureIndex;
pub use specialization::{
    GeneratedCaptureSemanticType, GeneratedCaptureSpecialization,
    GeneratedCaptureSpecializationIdentity,
};
use specialization::{normalize_semantic_presentations, specialization_for};

/// C1 query diagnostic: compiler artifacts carrying one structural capture
/// occurrence identity disagree. Tooling must stop rather than choose one.
pub const GENERATED_CAPTURE_ARTIFACT_CONFLICT_CODE: &str = "C0911";

/// Capture descriptors in this query are present only on generated nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GeneratedCaptureStage {
    GeneratedOnly,
}

/// Exact authored locations linked by one capture identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GeneratedCaptureSourceMap {
    binding: SourceAnchor,
    declaration: SourceAnchor,
    uses: Vec<SourceAnchor>,
}

impl GeneratedCaptureSourceMap {
    pub fn binding(&self) -> SourceAnchor {
        self.binding
    }

    pub fn declaration(&self) -> SourceAnchor {
        self.declaration
    }

    pub fn uses(&self) -> &[SourceAnchor] {
        &self.uses
    }

    fn role_at(&self, file_id: u16, offset: usize) -> Option<CaptureSiteRole> {
        if self.declaration.contains(file_id, offset) {
            return Some(CaptureSiteRole::Declaration);
        }
        if self
            .uses
            .iter()
            .any(|use_site| use_site.contains(file_id, offset))
        {
            return Some(CaptureSiteRole::Use);
        }
        self.binding
            .contains(file_id, offset)
            .then_some(CaptureSiteRole::Binding)
    }
}

/// Immutable projection of one validated generated capture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedCaptureDescriptorView {
    identity: GeneratedCaptureBindingIdentity,
    occurrence_identity: GeneratedCaptureOccurrenceIdentity,
    display_name: String,
    mode: CaptureMode,
    specializations: Vec<GeneratedCaptureSpecialization>,
    owner_display: String,
    owner_node_path: String,
    application: Option<SourceAnchor>,
    source_map: Option<GeneratedCaptureSourceMap>,
}

impl GeneratedCaptureDescriptorView {
    /// Identity of the captured binding. References join every occurrence
    /// carrying this identity.
    pub fn identity(&self) -> &GeneratedCaptureBindingIdentity {
        &self.identity
    }

    /// Identity of this exact capture clause/body occurrence. Hover and
    /// artifact-conflict checks remain occurrence-specific.
    pub fn occurrence_identity(&self) -> &GeneratedCaptureOccurrenceIdentity {
        &self.occurrence_identity
    }

    /// Diagnostic prose only; never use this spelling as a lookup key.
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    pub fn mode(&self) -> CaptureMode {
        self.mode
    }

    /// Every exact compiler specialization of this authored occurrence,
    /// sorted by structural typed identity rather than compilation order.
    pub fn specializations(&self) -> &[GeneratedCaptureSpecialization] {
        &self.specializations
    }

    /// The exact type when every specialization agrees. Generic occurrences
    /// with multiple semantic types return `None`; callers must display the
    /// explicit specialization set rather than choosing one.
    pub fn uniform_capture_type(&self) -> Option<&GeneratedCaptureSemanticType> {
        let first = self.specializations.first()?.capture_type();
        self.specializations
            .iter()
            .all(|specialization| specialization.capture_type() == first)
            .then_some(first)
    }

    pub fn stage(&self) -> GeneratedCaptureStage {
        GeneratedCaptureStage::GeneratedOnly
    }

    pub fn owner_display(&self) -> &str {
        &self.owner_display
    }

    pub fn owner_node_path(&self) -> &str {
        &self.owner_node_path
    }

    pub fn application(&self) -> Option<SourceAnchor> {
        self.application
    }

    pub fn source_map(&self) -> Option<&GeneratedCaptureSourceMap> {
        self.source_map.as_ref()
    }

    fn same_occurrence_contract(&self, other: &Self) -> bool {
        self.identity == other.identity
            && self.occurrence_identity == other.occurrence_identity
            && self.display_name == other.display_name
            && self.mode == other.mode
            && self.owner_display == other.owner_display
            && self.owner_node_path == other.owner_node_path
            && self.application == other.application
            && self.source_map == other.source_map
    }

    fn merge_specialization(
        &mut self,
        specialization: GeneratedCaptureSpecialization,
    ) -> Result<(), ()> {
        if let Some(existing) = self
            .specializations
            .iter_mut()
            .find(|existing| existing.identity() == specialization.identity())
        {
            return existing.merge_diagnostic_presentation(&specialization);
        }
        self.specializations.push(specialization);
        self.specializations.sort_by(|left, right| {
            left.identity()
                .canonical_descriptor()
                .cmp(&right.identity().canonical_descriptor())
        });
        Ok(())
    }
}

/// Whether the query matched the explicit capture declaration or a body use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureSiteRole {
    Declaration,
    Use,
    Binding,
}

/// Identity-bearing position result. The descriptor remains owned by the
/// query; consumers receive a borrow and cannot mutate compiler state.
#[derive(Debug, Clone)]
pub struct GeneratedCaptureSite<'query> {
    captures: Vec<&'query GeneratedCaptureDescriptorView>,
    role: CaptureSiteRole,
}

impl<'query> GeneratedCaptureSite<'query> {
    /// All exact generated occurrences mapped to this authored position.
    /// One annotation template can legitimately be applied more than once,
    /// so callers must aggregate this set rather than choose an arbitrary
    /// compiler artifact.
    pub fn captures(&self) -> &[&'query GeneratedCaptureDescriptorView] {
        &self.captures
    }

    pub fn role(&self) -> CaptureSiteRole {
        self.role
    }
}

/// Position lookup distinguishes ordinary source from a poisoned generated
/// capture site. Consumers may fall through only for `None`; `Unavailable`
/// must suppress name-based answers that could expose a conflicting artifact.
#[derive(Debug, Clone)]
pub enum GeneratedCapturePosition<'query> {
    Available(GeneratedCaptureSite<'query>),
    Unavailable,
}

/// Stable query failure. A descriptor with an unavailable source map remains
/// in `captures()` for compiler-query inspection, but cannot answer a source
/// hover/navigation request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedCaptureQueryIssue {
    code: &'static str,
    message: String,
    anchor: Option<SourceAnchor>,
}

impl GeneratedCaptureQueryIssue {
    pub fn code(&self) -> &'static str {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    /// Best real authored anchor for this issue. C0911 may fall back from an
    /// application to a verified declaration/binder site; `None` means no
    /// real source authority exists at all.
    pub fn anchor(&self) -> Option<SourceAnchor> {
        self.anchor
    }
}

/// Complete immutable query result for one compilation. Binding identities
/// exposed through this result are join keys only within this query/compiler
/// session; only occurrence identity carries the stable generated-node
/// provenance contract.
#[derive(Debug, Clone, Default)]
pub struct GeneratedCaptureQuery {
    captures: Vec<GeneratedCaptureDescriptorView>,
    issues: Vec<GeneratedCaptureQueryIssue>,
    quarantined_source_maps: Vec<GeneratedCaptureSourceMap>,
}

impl GeneratedCaptureQuery {
    pub fn captures(&self) -> &[GeneratedCaptureDescriptorView] {
        &self.captures
    }

    pub fn issues(&self) -> &[GeneratedCaptureQueryIssue] {
        &self.issues
    }

    /// Every authored capture occurrence of one compiler-issued binding in
    /// this query's compiler session. `identity` must originate from this
    /// query; it is not a persistent or cross-compilation key.
    /// Module bindings join by `(file, module slot)` across generated owners;
    /// locals retain their expansion and structural owner scope.
    pub fn captures_for_binding<'query>(
        &'query self,
        identity: &GeneratedCaptureBindingIdentity,
    ) -> impl Iterator<Item = &'query GeneratedCaptureDescriptorView> + 'query {
        let identity = identity.clone();
        self.captures
            .iter()
            .filter(move |capture| capture.identity == identity)
    }

    /// Resolve every exact occurrence mapped to an authored capture site.
    /// Declaration beats forwarding use, and use beats the originating
    /// binder, but the returned set retains every matching application.
    pub fn capture_at(&self, file_id: u16, offset: usize) -> Option<GeneratedCapturePosition<'_>> {
        if self
            .quarantined_source_maps
            .iter()
            .any(|source_map| source_map.role_at(file_id, offset).is_some())
        {
            return Some(GeneratedCapturePosition::Unavailable);
        }

        let mut captures = Vec::new();
        let mut role: Option<CaptureSiteRole> = None;
        for capture in &self.captures {
            let Some(candidate_role) = capture
                .source_map()
                .and_then(|source_map| source_map.role_at(file_id, offset))
            else {
                continue;
            };
            captures.push(capture);
            if role.is_none_or(|current| candidate_role.precedence() > current.precedence()) {
                role = Some(candidate_role);
            }
        }
        Some(GeneratedCapturePosition::Available(GeneratedCaptureSite {
            captures,
            role: role?,
        }))
    }
}

impl CaptureSiteRole {
    const fn precedence(self) -> u8 {
        match self {
            Self::Declaration => 3,
            Self::Use => 2,
            Self::Binding => 1,
        }
    }
}

impl BytecodeCompiler {
    /// Project the validated capture packs through the compiler-owned query.
    ///
    /// `source_program` is used only as a structural source-map verifier. It
    /// never produces identity or capture semantics; those come exclusively
    /// from the compiler-issued pack.
    pub fn generated_capture_query(&self, source_program: &Program) -> GeneratedCaptureQuery {
        if !self.generated_queries_available() {
            return GeneratedCaptureQuery::default();
        }
        GeneratedCaptureQuery::from_compiler(self, source_program)
    }
}

impl GeneratedCaptureQuery {
    fn from_compiler(compiler: &BytecodeCompiler, source_program: &Program) -> Self {
        let source_index = AuthoredCaptureIndex::build(source_program);
        let mut accumulator = CaptureAccumulator::new();
        let mut issues = Vec::new();

        for pack in &compiler.closure_capture_packs {
            let Some(origin) = pack.origin.as_ref() else {
                continue;
            };
            let (file_id, application_span) = origin.anchor();
            let application = SourceAnchor::new(file_id, application_span).ok();

            for (descriptor_ordinal, descriptor) in pack.descriptors.iter().enumerate() {
                let Some(mode) = descriptor.declared else {
                    continue;
                };
                let occurrence_identity = GeneratedCaptureOccurrenceIdentity {
                    node: GeneratedNodeKey::from_origin(origin),
                    descriptor_ordinal,
                };
                let source_map = source_index.source_map_for(compiler, origin, descriptor);
                if application.is_none() {
                    accumulator.poison(
                        occurrence_identity,
                        source_map,
                        None,
                        format!(
                            "generated capture '{}' in node {} has no real application anchor",
                            descriptor.name,
                            origin.render_path(),
                        ),
                    );
                    continue;
                }
                let Some(lineage) = descriptor.binding_lineage.as_ref() else {
                    accumulator.poison(
                        occurrence_identity,
                        source_map,
                        application,
                        format!(
                            "generated capture '{}' in node {} has no canonical binding lineage",
                            descriptor.name,
                            origin.render_path(),
                        ),
                    );
                    continue;
                };
                let identity = GeneratedCaptureBindingIdentity::from_binding_lineage(lineage);
                let specialization = match specialization_for(pack, descriptor_ordinal) {
                    Ok(specialization) => specialization,
                    Err(reason) => {
                        accumulator.poison(
                            occurrence_identity,
                            source_map,
                            application,
                            format!(
                                "generated capture '{}' cannot establish a structural specialization identity: {reason}",
                                descriptor.name,
                            ),
                        );
                        continue;
                    }
                };
                if source_map.is_none() {
                    // ADR-009 E2 #18 slice 5 (Part B): post-U03 deletion, the
                    // source-string reparse route was the SOLE producer of a
                    // validated capture with no structural source map. A missing
                    // source map here is now an INVARIANT VIOLATION, not an honest
                    // degraded-mode state — surface it LOUDLY through the same
                    // poison path the sibling evidence-gap failures above use (a
                    // [C0911] conflict quarantine that stops tooling from treating
                    // the capture as an ordinary rename), NEVER a silent
                    // pass-through and NEVER a degraded descriptor view.
                    accumulator.poison(
                        occurrence_identity,
                        source_map,
                        application,
                        "generated capture with no source map post-U03: structural source-map validation failed"
                            .to_string(),
                    );
                    continue;
                }
                let view = GeneratedCaptureDescriptorView {
                    identity: identity.clone(),
                    occurrence_identity: occurrence_identity.clone(),
                    display_name: descriptor.name.clone(),
                    mode,
                    specializations: vec![specialization],
                    owner_display: origin.owner_display().to_string(),
                    owner_node_path: origin.render_path(),
                    application,
                    source_map,
                };
                accumulator.insert(view);
            }
        }

        let mut aggregated = accumulator.finish();
        normalize_semantic_presentations(&mut aggregated.captures);
        issues.extend(aggregated.issues);
        issues.sort_by(|left, right| {
            (
                left.code,
                left.message.as_str(),
                left.anchor.map(source_anchor_key),
            )
                .cmp(&(
                    right.code,
                    right.message.as_str(),
                    right.anchor.map(source_anchor_key),
                ))
        });
        issues.dedup();
        Self {
            captures: aggregated.captures,
            issues,
            quarantined_source_maps: aggregated.quarantined_source_maps,
        }
    }
}

fn source_anchor_key(anchor: SourceAnchor) -> (u16, usize, usize) {
    (anchor.file_id(), anchor.span().start, anchor.span().end)
}
