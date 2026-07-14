//! Read-only compiler query for validated generated capture descriptors.
//!
//! Capture identity comes only from the compiler-issued expansion origin and
//! resolved slot carried by the compiler's capture pack. Source spans are presentation
//! data: this module publishes them only when every span round-trips through
//! a structural AST node in the caller's source program. Reparsed generated
//! strings therefore remain represented as typed descriptors in the compiler
//! query but never gain invented source locations.

use std::collections::HashMap;

use crate::compiler::{BytecodeCompiler, SourceAnchor};
use shape_ast::ast::{CaptureMode, Program};

mod identity;
mod source_index;
mod specialization;
pub use identity::{
    GeneratedCaptureBindingIdentity, GeneratedCaptureOccurrenceIdentity, GeneratedCaptureSlot,
};
use source_index::AuthoredCaptureIndex;
use specialization::specialization_for;
pub use specialization::{GeneratedCaptureSpecialization, GeneratedCaptureSpecializationIdentity};

/// C1 query diagnostic: a validated capture exists, but its generated AST
/// offsets do not have an exact structural mapping into the source program.
pub const GENERATED_CAPTURE_SOURCE_UNAVAILABLE_CODE: &str = "C0910";

/// C1 query diagnostic: compiler artifacts carrying one structural capture
/// occurrence identity disagree. Tooling must stop rather than choose one.
pub const GENERATED_CAPTURE_ARTIFACT_CONFLICT_CODE: &str = "C0911";

/// Capture descriptors in this query are present only on generated nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GeneratedCaptureStage {
    GeneratedOnly,
}

/// Exact authored locations linked by one capture identity.
#[derive(Debug, Clone, PartialEq, Eq)]
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

    fn contains_declaration_or_use(&self, file_id: u16, offset: usize) -> Option<CaptureSiteRole> {
        if self.declaration.contains(file_id, offset) {
            return Some(CaptureSiteRole::Declaration);
        }
        self.uses
            .iter()
            .any(|use_site| use_site.contains(file_id, offset))
            .then_some(CaptureSiteRole::Use)
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
    /// with multiple concrete types return `None`; callers must display the
    /// explicit specialization set rather than choosing one.
    pub fn uniform_capture_type(&self) -> Option<&shape_value::v2::ConcreteType> {
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
            .iter()
            .find(|existing| existing.identity() == specialization.identity())
        {
            return (existing == &specialization).then_some(()).ok_or(());
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
}

/// Identity-bearing position result. The descriptor remains owned by the
/// query; consumers receive a borrow and cannot mutate compiler state.
#[derive(Debug, Clone, Copy)]
pub struct GeneratedCaptureSite<'query> {
    capture: &'query GeneratedCaptureDescriptorView,
    role: CaptureSiteRole,
}

impl<'query> GeneratedCaptureSite<'query> {
    pub fn capture(&self) -> &'query GeneratedCaptureDescriptorView {
        self.capture
    }

    pub fn role(&self) -> CaptureSiteRole {
        self.role
    }
}

/// Stable query failure. A descriptor with an unavailable source map remains
/// in `captures()` for compiler-query inspection, but cannot answer a source
/// hover/navigation request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedCaptureQueryIssue {
    code: &'static str,
    message: String,
    application: Option<SourceAnchor>,
}

impl GeneratedCaptureQueryIssue {
    pub fn code(&self) -> &'static str {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn application(&self) -> Option<SourceAnchor> {
        self.application
    }
}

/// Complete immutable query result for one compilation.
#[derive(Debug, Clone, Default)]
pub struct GeneratedCaptureQuery {
    captures: Vec<GeneratedCaptureDescriptorView>,
    issues: Vec<GeneratedCaptureQueryIssue>,
}

impl GeneratedCaptureQuery {
    pub fn captures(&self) -> &[GeneratedCaptureDescriptorView] {
        &self.captures
    }

    pub fn issues(&self) -> &[GeneratedCaptureQueryIssue] {
        &self.issues
    }

    /// Every authored capture occurrence of one compiler-issued binding.
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

    /// Resolve only an explicit declaration or captured body use. The outer
    /// binding itself deliberately falls through to ordinary source tooling.
    pub fn capture_at(&self, file_id: u16, offset: usize) -> Option<GeneratedCaptureSite<'_>> {
        let mut resolved_use = None;
        for capture in &self.captures {
            let Some(role) = capture
                .source_map()
                .and_then(|source_map| source_map.contains_declaration_or_use(file_id, offset))
            else {
                continue;
            };
            let site = GeneratedCaptureSite { capture, role };
            if role == CaptureSiteRole::Declaration {
                return Some(site);
            }
            resolved_use.get_or_insert(site);
        }
        resolved_use
    }
}

impl BytecodeCompiler {
    /// Project the validated capture packs through the compiler-owned query.
    ///
    /// `source_program` is used only as a structural source-map verifier. It
    /// never produces identity or capture semantics; those come exclusively
    /// from the compiler-issued pack.
    pub fn generated_capture_query(&self, source_program: &Program) -> GeneratedCaptureQuery {
        GeneratedCaptureQuery::from_compiler(self, source_program)
    }
}

impl GeneratedCaptureQuery {
    fn from_compiler(compiler: &BytecodeCompiler, source_program: &Program) -> Self {
        let source_index = AuthoredCaptureIndex::build(source_program);
        let mut captures_by_occurrence: HashMap<
            GeneratedCaptureOccurrenceIdentity,
            GeneratedCaptureDescriptorView,
        > = HashMap::new();
        let mut issues = Vec::new();

        for pack in &compiler.closure_capture_packs {
            let Some(origin) = pack.origin.as_ref() else {
                continue;
            };
            let (file_id, application_span) = origin.anchor();
            let application = SourceAnchor::new(file_id, application_span).ok();
            if application.is_none() {
                issues.push(GeneratedCaptureQueryIssue {
                    code: GENERATED_CAPTURE_ARTIFACT_CONFLICT_CODE,
                    message: format!(
                        "[C0911] generated capture node {} has no real application anchor",
                        origin.render_path(),
                    ),
                    application: None,
                });
            }
            let mut owner_path = origin.node_path().to_vec();
            owner_path.pop();

            for (descriptor_ordinal, descriptor) in pack.descriptors.iter().enumerate() {
                let Some(mode) = descriptor.declared else {
                    continue;
                };
                let Some(target) = descriptor.target else {
                    issues.push(GeneratedCaptureQueryIssue {
                        code: GENERATED_CAPTURE_ARTIFACT_CONFLICT_CODE,
                        message: format!(
                            "[C0911] generated capture '{}' in node {} has no resolved binding slot",
                            descriptor.name,
                            origin.render_path(),
                        ),
                        application,
                    });
                    continue;
                };
                let identity = GeneratedCaptureBindingIdentity::from_capture_target(
                    origin.expansion_fingerprint(),
                    owner_path.clone(),
                    file_id,
                    target,
                );
                let occurrence_identity = GeneratedCaptureOccurrenceIdentity {
                    expansion_fingerprint: origin.expansion_fingerprint(),
                    file_id,
                    capture_node_path: origin.node_path().to_vec(),
                    descriptor_ordinal,
                };
                let specialization = match specialization_for(compiler, pack, descriptor_ordinal) {
                    Ok(specialization) => specialization,
                    Err(reason) => {
                        issues.push(GeneratedCaptureQueryIssue {
                            code: GENERATED_CAPTURE_ARTIFACT_CONFLICT_CODE,
                            message: format!(
                                "[C0911] generated capture '{}' in occurrence {} cannot establish a structural specialization identity: {reason}",
                                descriptor.name,
                                occurrence_identity.canonical_descriptor(),
                            ),
                            application,
                        });
                        continue;
                    }
                };
                let source_map = source_map_for(descriptor, mode, file_id, &source_index);
                if source_map.is_none() {
                    issues.push(GeneratedCaptureQueryIssue {
                        code: GENERATED_CAPTURE_SOURCE_UNAVAILABLE_CODE,
                        message: format!(
                            "[C0910] generated capture '{}' in node {} has no exact source map; its typed descriptor remains available through the compiler capture query, but source hover and navigation are unavailable",
                            descriptor.name,
                            origin.render_path(),
                        ),
                        application,
                    });
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
                match captures_by_occurrence.entry(occurrence_identity) {
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        entry.insert(view);
                    }
                    std::collections::hash_map::Entry::Occupied(mut entry) => {
                        let same_contract = entry.get().same_occurrence_contract(&view);
                        let specialization = view
                            .specializations
                            .into_iter()
                            .next()
                            .expect("candidate capture view has one specialization");
                        if !same_contract
                            || entry
                                .get_mut()
                                .merge_specialization(specialization)
                                .is_err()
                        {
                            issues.push(GeneratedCaptureQueryIssue {
                                code: GENERATED_CAPTURE_ARTIFACT_CONFLICT_CODE,
                                message: format!(
                                    "[C0911] generated capture artifacts disagree for occurrence {}",
                                    entry.key().canonical_descriptor(),
                                ),
                                application,
                            });
                        }
                    }
                }
            }
        }

        let mut captures: Vec<_> = captures_by_occurrence.into_values().collect();
        captures.sort_by(|left, right| {
            left.occurrence_identity
                .canonical_descriptor()
                .cmp(&right.occurrence_identity.canonical_descriptor())
        });
        issues.sort_by(|left, right| {
            (left.code, left.message.as_str()).cmp(&(right.code, right.message.as_str()))
        });
        issues.dedup();
        Self { captures, issues }
    }
}

fn source_map_for(
    descriptor: &super::CaptureDescriptor,
    mode: CaptureMode,
    file_id: u16,
    index: &AuthoredCaptureIndex,
) -> Option<GeneratedCaptureSourceMap> {
    let binding_span = descriptor.binding_span?;
    let declaration_span = descriptor.declaration_span?;
    // A successfully planned explicit descriptor always has a body use:
    // `plan_captures` rejects stale declarations as [C0901] before building a
    // CapturePack. Keep that compiler invariant explicit here; an empty list
    // in a supposedly validated pack is an unavailable/inconsistent artifact,
    // never a declaration-only source map invented by tooling.
    if descriptor.use_spans.is_empty()
        || !index.has_binding(&descriptor.name, binding_span)
        || !index.has_declaration(&descriptor.name, mode, declaration_span)
        || descriptor
            .use_spans
            .iter()
            .any(|span| !index.has_use(&descriptor.name, *span))
    {
        return None;
    }

    let binding = SourceAnchor::new(file_id, binding_span).ok()?;
    let declaration = SourceAnchor::new(file_id, declaration_span).ok()?;
    let mut uses: Vec<_> = descriptor
        .use_spans
        .iter()
        .filter_map(|span| SourceAnchor::new(file_id, *span).ok())
        .collect();
    uses.sort_by_key(|anchor| (anchor.span().start, anchor.span().end));
    uses.dedup();
    (uses.len() == descriptor.use_spans.len()).then_some(GeneratedCaptureSourceMap {
        binding,
        declaration,
        uses,
    })
}
