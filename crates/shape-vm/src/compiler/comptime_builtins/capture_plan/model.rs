use super::*;
use shape_ast::ast::{GeneratedExpansionFingerprint, GeneratedNodePath};

/// Slot-keyed identity of a captured binding.
///
/// R1: the live path never keys a capture on a source name or a `Span`.
/// Generated AST parses from offset 0, so spans collide across generated
/// closures — that was rejection finding (2) of the first C1 attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum CaptureTarget {
    /// Frame-local slot of the *enclosing* function.
    Local(u16),
    /// Module-binding slot (program lifetime).
    ModuleBinding(u16),
}

/// Canonical compiler-issued identity of the original captured binding.
///
/// Unlike [`CaptureTarget`], which is relative to the immediately enclosing
/// frame, this lineage survives forwarding through synthetic capture
/// parameters. No spelling or presentation span participates. Module bindings
/// deliberately exclude expansion ownership so every generated occurrence of
/// one `(file, module-slot)` binding joins; locals retain the structural
/// expansion owner and original slot so distinct cells cannot collide.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum CaptureBindingLineage {
    Local {
        expansion_fingerprint: GeneratedExpansionFingerprint,
        binding_owner_path: GeneratedNodePath,
        file_id: u16,
        slot: u16,
    },
    ModuleBinding {
        file_id: u16,
        slot: u16,
    },
}

impl CaptureBindingLineage {
    pub(crate) fn from_generated_capture(
        origin: &GeneratedNodeOrigin,
        binding_file_id: u16,
        target: CaptureTarget,
    ) -> Result<Self> {
        let closure_segment = origin
            .path()
            .typed_segments()
            .last()
            .ok_or_else(|| ShapeError::RuntimeError {
                message:
                    "internal compiler error: generated capture origin has no structural closure segment"
                        .to_string(),
                location: None,
            })?;
        let valid_closure_segment = closure_segment
            .as_str()
            .strip_prefix("closure:")
            .is_some_and(|index| !index.is_empty() && index.parse::<u32>().is_ok());
        if !valid_closure_segment {
            return Err(ShapeError::RuntimeError {
                message: format!(
                    "internal compiler error: generated capture origin ends in invalid structural segment '{closure_segment}'"
                ),
                location: None,
            });
        }
        let binding_owner_path = origin
            .path()
            .parent()
            .expect("a validated terminal closure segment has a structural parent path");
        match target {
            CaptureTarget::Local(slot) => Ok(Self::Local {
                expansion_fingerprint: origin.identity().expansion(),
                binding_owner_path,
                file_id: binding_file_id,
                slot,
            }),
            CaptureTarget::ModuleBinding(slot) => Ok(Self::ModuleBinding {
                file_id: binding_file_id,
                slot,
            }),
        }
    }
}

/// Frozen semantic type of a captured binding.
///
/// This is independent of [`ConcreteType`], whose closure/function IDs and
/// pointer fallbacks are runtime ABI carriers, not semantic identity. The
/// value is issued only by the ADR-009 semantic freeze's single canonicalizer;
/// no inference carrier or second type descriptor is retained here.
#[derive(Debug, Clone)]
pub(crate) struct CaptureSemanticType(super::super::semantic_freeze::FrozenSemanticTypeProjection);

impl CaptureSemanticType {
    pub(crate) fn from_semantic_candidate(
        candidate: &shape_runtime::type_system::SemanticTypeCandidate,
        freeze: &super::super::semantic_freeze::FreezeOverlay,
    ) -> std::result::Result<Self, String> {
        let annotation = freeze.semantic_candidate_annotation(candidate)?;
        freeze.canonicalize_type_projection(&annotation).map(Self)
    }

    pub(crate) fn category(&self) -> shape_runtime::comptime_reflection::FrozenTypeCategory {
        self.0.category()
    }

    pub(crate) fn identity_components(&self) -> (i64, i64) {
        let identity = self.0.identity();
        (identity.high, identity.low)
    }

    /// Canonical freeze descriptor for diagnostic rendering only. It must
    /// never be parsed or used as an identity key.
    pub(crate) fn presentation(&self) -> &str {
        self.0.presentation()
    }
}

impl PartialEq for CaptureSemanticType {
    fn eq(&self, other: &Self) -> bool {
        self.category() == other.category()
            && self.identity_components() == other.identity_components()
    }
}

impl Eq for CaptureSemanticType {}

impl std::hash::Hash for CaptureSemanticType {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(&self.category().catalog_ordinal(), state);
        std::hash::Hash::hash(&self.identity_components(), state);
    }
}

impl PartialOrd for CaptureSemanticType {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CaptureSemanticType {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (
            self.category().catalog_ordinal(),
            self.identity_components(),
        )
            .cmp(&(
                other.category().catalog_ordinal(),
                other.identity_components(),
            ))
    }
}

/// Stable reason why a capture descriptor cannot expose an exact semantic
/// type. The detail is diagnostic-only and never participates in identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum CaptureSemanticIssueKind {
    MissingSemanticFreeze,
    MissingInferenceFact,
    InferenceUnavailable,
    InferenceConflict,
    FreezeRejected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CaptureSemanticIssue {
    kind: CaptureSemanticIssueKind,
    detail: String,
}

impl CaptureSemanticIssue {
    pub(crate) fn new(kind: CaptureSemanticIssueKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
        }
    }

    pub(crate) fn kind(&self) -> CaptureSemanticIssueKind {
        self.kind
    }

    pub(crate) fn detail(&self) -> &str {
        &self.detail
    }
}

/// Exact semantic capture evidence, or an explicit reason it cannot be used.
/// Missing and conflicting evidence are never collapsed into `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CaptureSemanticEvidence {
    Exact(CaptureSemanticType),
    Unavailable(CaptureSemanticIssue),
    Conflict(CaptureSemanticIssue),
}

impl CaptureSemanticEvidence {
    pub(crate) fn unavailable(kind: CaptureSemanticIssueKind, detail: impl Into<String>) -> Self {
        Self::Unavailable(CaptureSemanticIssue::new(kind, detail))
    }

    pub(crate) fn conflict(kind: CaptureSemanticIssueKind, detail: impl Into<String>) -> Self {
        Self::Conflict(CaptureSemanticIssue::new(kind, detail))
    }
}

/// Everything the selector is allowed to look at.
///
/// Gathered once, in the enclosing function's scope, before the closure body
/// is compiled (`compile_function` re-points the type tracker at the closure's
/// own slots). One value per capture, in `captured_vars` declaration order.
#[derive(Debug, Clone)]
pub(crate) struct CaptureBindingFacts {
    /// Source spelling. Diagnostic prose and emission bookkeeping ONLY — the
    /// selector reads `target`, never this.
    pub(crate) name: String,
    /// `None` when the capture resolves to neither a frame local nor a module
    /// binding. Believed unreachable (`collect_outer_scope_vars` is exactly
    /// locals ∪ module bindings, and captures are drawn from it), but the
    /// pre-fusion selector had a live arm for it and this is a
    /// behaviour-preserving fusion, so the arm is reproduced rather than
    /// panicked on. The declared path (slice 3) rejects `None` by name.
    pub(crate) target: Option<CaptureTarget>,
    /// Exact declaration span for the compiler-issued target, when the target
    /// originates in authored source. Presentation evidence only: capture
    /// identity and lowering remain slot-keyed through `target`.
    pub(crate) binding_span: Option<Span>,
    /// Canonical original binding identity, when this fact was forwarded from
    /// an inherited generated capture parameter. Initial generated captures
    /// mint lineage only when their descriptor is built with node provenance.
    pub(crate) binding_lineage: Option<CaptureBindingLineage>,
    /// Source-file namespace in which `target` was resolved. This comes from
    /// the compiler's active binding/module tables, never from the generated
    /// expansion's application anchor.
    pub(crate) binding_file_id: u16,
    /// Exact, fully resolved semantic type from canonical binding inference,
    /// or a typed unavailable/conflict state. Never an ABI carrier fallback.
    pub(crate) semantic_type: CaptureSemanticEvidence,
    /// `binding_semantics_for_name(..).ownership_class`.
    pub(crate) ownership: Option<BindingOwnershipClass>,
    /// `mir_storage_class_for_slot` for local targets (ADR-006 §4.2).
    pub(crate) storage: Option<BindingStorageClass>,
    /// The closure body writes through this capture
    /// (`mutated_captures` ∪ `collect_static_mut_self_container_captures`).
    pub(crate) mutated: bool,
    /// `boxed_locals` witness.
    pub(crate) boxed: bool,
    /// `shared_locals` witness (a sibling closure already promoted the local
    /// to a `SharedCell`).
    pub(crate) witness_shared_local: bool,
    /// `shared_module_binding_contains` witness.
    pub(crate) witness_shared_module_binding: bool,
    /// `owned_mutable_locals` witness (a sibling closure already classified
    /// this local `OwnedMutable`).
    pub(crate) witness_owned_mutable_local: bool,
    /// This target is a compiler-issued leading parameter inherited from an
    /// enclosing capture pack. Even when lineage is unavailable, a nested
    /// pack must not mint a replacement identity from this immediate slot.
    pub(crate) inherited_capture_parameter: bool,
    /// Structural evidence from the enclosing closure's capture pack: this
    /// synthetic parameter slot carries the raw `Arc<SharedCell>` pointer.
    /// Unlike the legacy witness sets, this is slot-keyed and survives nested
    /// closure compilation without reclassifying the parameter by name.
    pub(crate) inherited_shared_cell: bool,
}

impl CaptureBindingFacts {
    pub(super) fn is_local(&self) -> bool {
        matches!(self.target, Some(CaptureTarget::Local(_)))
    }

    pub(super) fn is_module_binding(&self) -> bool {
        matches!(self.target, Some(CaptureTarget::ModuleBinding(_)))
    }
}

/// How the *closure body* reaches a capture.
///
/// This is the old `mutable_flags[i]` boolean, refined into the four
/// dispositions the emitter actually distinguishes. There is no residual /
/// unknown / fallback arm: every capture lands in exactly one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CaptureAccess {
    /// Leading immutable closure param — the value is snapshot by copy at
    /// `MakeClosure` time. (`mutable_flags[i] == false`.)
    Param,
    /// `Load/StoreOwnedMutableCapture` — the closure owns a `Box`-backed cell.
    OwnedMutableCell,
    /// `Load/StoreSharedCapture` — the closure holds an `Arc<SharedCell>` share.
    SharedCell,
    /// The honest name for the pre-existing inference residual: the body needs
    /// cell access (`mutable_flags[i] == true`) but the classifier landed on
    /// the `Immutable` kind, so the layout mask stays clear and the body falls
    /// back to legacy `LoadClosure`/`StoreClosure`.
    ///
    /// It is reachable on the INFERRED path only — reproduced here bit-for-bit
    /// so the fusion is behaviour-preserving. The declared path (slice 3) must
    /// never produce it; it is a hard rejection there, never a fallback arm.
    MutableCell,
}

/// One descriptor's structural evidence threaded into the synthetic leading
/// parameter of its recursively compiled closure body.
///
/// Descriptor ordinal selects the local slot. Lineage and frozen semantic type
/// are copied unchanged for every capture mode; `binding_span` remains
/// presentation-only and never participates in selection or classification.
#[derive(Debug, Clone)]
pub(crate) struct CaptureParameterEvidence {
    pub(crate) access: CaptureAccess,
    pub(crate) binding_span: Option<Span>,
    pub(crate) binding_lineage: Option<CaptureBindingLineage>,
    pub(crate) semantic_type: CaptureSemanticEvidence,
}

impl CaptureAccess {
    /// The old `mutable_flags[i]`: "the body must reach this capture through
    /// the frame's capture slots rather than a leading param".
    pub(crate) fn needs_cell(self) -> bool {
        !matches!(self, CaptureAccess::Param)
    }
}

/// The single selector output: what the layout says AND how the body reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CapturePlan {
    kind: CaptureKind,
    access: CaptureAccess,
}

impl CapturePlan {
    pub(super) const fn new(kind: CaptureKind, access: CaptureAccess) -> Self {
        Self { kind, access }
    }

    pub(crate) fn kind(&self) -> CaptureKind {
        self.kind
    }

    pub(crate) fn access(&self) -> CaptureAccess {
        self.access
    }

    pub(crate) fn needs_cell(&self) -> bool {
        self.access.needs_cell()
    }
}

/// One capture's full record on the compile path.
///
/// R1: constructed on the LIVE path (see `compile_expr_closure`), not behind a
/// `#[allow(dead_code)]`. `lowered` is what the emitted `ClosureLayout`
/// carries; the R2 equivalence test asserts that against the EMITTED artifact
/// (`program.closure_function_layouts[fid].capture_storage_kind(i)`), never
/// against this table.
#[derive(Debug, Clone)]
pub(crate) struct CaptureDescriptor {
    pub(crate) index: u16,
    pub(crate) target: Option<CaptureTarget>,
    pub(crate) capture_type: ConcreteType,
    /// ADR-009 C1 (slice 3): the DECLARED mode, when the closure carried a
    /// capture clause. `Some(mode)` means `lowered` came from
    /// [`lower_declared`] and the declaration DROVE emission; `None` means it
    /// came from [`infer_plan`].
    ///
    /// Per user rulings 1 + 2 the declared word and the lowered kind can never
    /// disagree (`move`→Immutable/OwnedMutable, `share`→Shared, and every other
    /// pairing is a named rejection), so this is provenance, not a second
    /// opinion. There is deliberately NO `declared != lowered` gap to surface.
    pub(crate) declared: Option<CaptureMode>,
    pub(crate) lowered: CaptureKind,
    pub(crate) access: CaptureAccess,
    /// Source declaration ownership, retained so the final artifact boundary
    /// can independently re-derive the exact `move` kind. This is not inferred
    /// from the emitted layout.
    pub(crate) ownership: Option<BindingOwnershipClass>,
    pub(crate) storage: Option<BindingStorageClass>,
    /// Whether the source slot is a synthetic parameter carrying an inherited
    /// raw SharedCell carrier. Retained so artifact/unit proofs can distinguish
    /// true nested sharing from a coincidental local `var` classification.
    pub(crate) inherited_shared_cell: bool,
    /// Canonical original binding identity. Every nested capture mode copies
    /// this unchanged from the outer descriptor by capture ordinal.
    pub(crate) binding_lineage: Option<CaptureBindingLineage>,
    /// Fully resolved semantic binding type, or a typed unavailable/conflict
    /// state. ABI emission continues to use `capture_type`; tooling must use
    /// this evidence or refuse.
    pub(crate) semantic_type: CaptureSemanticEvidence,
    /// Exact authored declaration span for the captured binding, when one is
    /// available. This never participates in capture identity.
    pub(crate) binding_span: Option<Span>,
    /// Exact capture-clause identifier span. Generated strings whose offsets
    /// do not round-trip to authored syntax deliberately retain an unavailable
    /// source map rather than acquiring a guessed location.
    pub(crate) declaration_span: Option<Span>,
    /// Lexically resolved body-use spans from the canonical environment walk.
    /// Presentation evidence only; names and spans never select the target.
    pub(crate) use_spans: Vec<Span>,
    /// Source spelling — diagnostics only.
    pub(crate) name: String,
}

/// The per-closure capture record, keyed by `func_idx` (R3).
#[derive(Debug, Clone)]
pub(crate) struct CapturePack {
    /// Closure function index. Unique per compiled closure, including per
    /// monomorphized instantiation. NEVER a `Span`.
    pub(crate) closure: u16,
    /// ADR-009 C1 (slice 2) / R3 — the closure's PROVENANCE, when it is a
    /// generated node: the owning expansion's 128-bit fingerprint plus the
    /// structured node path (`extend:Job/method:read/closure:0`). `None` for an
    /// ordinary source closure. Read on the live path by
    /// [`CapturePack::generated_note`], which attributes a capture diagnostic
    /// raised inside generated code to the expansion that produced it.
    pub(crate) origin: Option<GeneratedNodeOrigin>,
    /// Exact whole-callable semantics for specialization/tooling, or a typed
    /// reason they are unavailable/conflicting. ABI execution never reads it.
    pub(crate) callable_semantic_evidence: CallableSemanticEvidence,
    pub(crate) descriptors: Vec<CaptureDescriptor>,
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod model_tests;
