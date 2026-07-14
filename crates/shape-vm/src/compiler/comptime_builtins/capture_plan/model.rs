use super::*;

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
    pub(crate) descriptors: Vec<CaptureDescriptor>,
}
