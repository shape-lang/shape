//! ADR-009 ticket C1 — **the one capture selector**.
//!
//! Before this module, closure-capture emission was driven by TWO coupled
//! vectors built independently inside `compile_expr_closure`:
//!
//!   * `mutable_flags: Vec<bool>` — a 5-way OR (mutated ∨ boxed ∨
//!     shared-local witness ∨ shared-module-binding witness ∨ `var`), read by
//!     `Function.mutable_captures`, the OwnedMutable escape veto, the
//!     body-emission maps, and the per-capture push emission; and
//!   * `capture_kinds: Vec<CaptureKind>` — which short-circuited to
//!     `Immutable` whenever `!mutable_flags[i]`, and otherwise re-derived the
//!     binding's ownership class, feeding `ClosureLayout` (and therefore the
//!     three capture masks the VM's `op_make_closure`, the JIT's
//!     `emit_heap_closure`, and `release_typed_closure` all dispatch on).
//!
//! Two coupled producers is exactly the seam through which C1's first attempt
//! defected: a declared capture mode set only `capture_kinds`, flipping the
//! layout mask while the body still read the capture as a leading immutable
//! param. This module collapses both into ONE producer — [`infer_plan`] — and
//! every former reader becomes a view on the resulting [`CapturePlan`]:
//!
//!   * `plan.access` answers "does the body reach this capture through a cell?"
//!     (the old `mutable_flags[i]`), and *which* cell discipline;
//!   * `plan.kind` answers "what does the emitted `ClosureLayout` say?"
//!     (the old `capture_kinds[i]`).
//!
//! The two can no longer disagree, because they are computed together from one
//! [`CaptureBindingFacts`] value.
//!
//! ## The K1 mechanical gate
//!
//! `CaptureKind::{Immutable,OwnedMutable,Shared}` may be *named* in exactly one
//! file in the bytecode compiler: this one. `scripts/check-no-dynamic.sh`
//! fails the build on any second producer, and
//! [`tests::capture_kind_is_constructed_in_exactly_one_compiler_file`] pins the
//! same invariant at the unit level. Without that gate, "one selector" is a
//! code-review norm — and a norm is what failed here the first time.
//!
//! ## Identity is structural (R1/R3)
//!
//! [`CaptureTarget`] is slot-keyed (`Local(u16)` / `ModuleBinding(u16)`), never
//! a source name and never a `Span`. [`CapturePack`] is keyed by the closure's
//! `func_idx`, which is unique per compiled closure *including* per
//! monomorphized instantiation. Names survive only as diagnostic prose in
//! [`CaptureBindingFacts::name`].

use shape_value::v2::closure_layout::CaptureKind;
use shape_value::v2::concrete_type::ConcreteType;

use crate::compiler::BytecodeCompiler;
use crate::type_tracking::{BindingOwnershipClass, BindingStorageClass};

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
    fn is_local(&self) -> bool {
        matches!(self.target, Some(CaptureTarget::Local(_)))
    }

    fn is_module_binding(&self) -> bool {
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

    /// True when this capture's kind flips one of the layout's non-heap mask
    /// bits (`owned_mutable_capture_mask` / `shared_capture_mask`) — i.e. the
    /// closure's `ClosureTypeId` must be re-interned kinds-aware.
    pub(crate) fn kind_is_cell_backed(&self) -> bool {
        !matches!(self.kind, CaptureKind::Immutable)
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
    pub(crate) lowered: CaptureKind,
    pub(crate) access: CaptureAccess,
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
    pub(crate) descriptors: Vec<CaptureDescriptor>,
}

impl CapturePack {
    /// The emitted layout's `capture_kinds` vector.
    pub(crate) fn kinds(&self) -> Vec<CaptureKind> {
        self.descriptors.iter().map(|d| d.lowered).collect()
    }

    pub(crate) fn len(&self) -> usize {
        self.descriptors.len()
    }
}

/// THE SELECTOR. The only place in the bytecode compiler that decides a
/// `CaptureKind`.
///
/// The logic is the pre-fusion inference, verbatim: `mutable_flags[i]`'s 5-way
/// OR followed by `capture_kinds[i]`'s ownership dispatch, with the
/// `!mutable_flags[i] ⇒ Immutable` short-circuit preserved as the `Param` arm.
/// [`tests::fused_plan_matches_legacy_pair_across_cross_product`] pins the
/// equivalence over the full fact cross-product.
pub(crate) fn infer_plan(facts: &CaptureBindingFacts) -> CapturePlan {
    // Pre-fusion `mutable_flags[i]` (closures.rs, 5-way OR). A capture needs
    // cell access if the closure mutates it, if the source slot is already
    // boxed, if a sibling closure already promoted it to a shared cell, or if
    // the source binding is `var` (`Flexible`) — a read-only closure over a
    // `var` must observe the same cell as a mutating sibling, not snapshot it.
    let is_flexible_capture =
        matches!(facts.ownership, Some(BindingOwnershipClass::Flexible)) && facts.target.is_some();
    let needs_cell = facts.mutated
        || facts.boxed
        || facts.witness_shared_local
        || facts.witness_shared_module_binding
        || is_flexible_capture;

    if !needs_cell {
        return CapturePlan {
            kind: CaptureKind::Immutable,
            access: CaptureAccess::Param,
        };
    }

    let is_local = facts.is_local();
    let is_module_binding = facts.is_module_binding();

    // Pre-fusion `capture_kinds[i]` ownership dispatch.
    let kind = match facts.ownership {
        // `let mut` local: moved by value into the closure's Box cell.
        Some(BindingOwnershipClass::OwnedMutable) if is_local => CaptureKind::OwnedMutable,
        // `let mut` at module scope is program-lifetime — there is no move, so
        // it rides the shared-cell pipeline and mutations propagate outward.
        Some(BindingOwnershipClass::OwnedMutable) if is_module_binding => CaptureKind::Shared,
        Some(BindingOwnershipClass::OwnedMutable) => CaptureKind::Immutable,
        // `var`: shared ownership, local or module binding alike.
        Some(BindingOwnershipClass::Flexible) if is_local || is_module_binding => {
            CaptureKind::Shared
        }
        Some(BindingOwnershipClass::Flexible) => CaptureKind::Immutable,
        // Ownership lookup can miss when a prior closure's `compile_function`
        // re-pointed the type tracker at its own slots. The persistent
        // witnesses recorded by earlier classification passes stand in, so a
        // sibling closure reclassifies the same way (a reclassification to
        // Immutable would null the layout mask and trip `op_make_closure`'s
        // layout-mismatch guard).
        _ if is_local && facts.witness_shared_local => CaptureKind::Shared,
        _ if is_local && facts.witness_owned_mutable_local => CaptureKind::OwnedMutable,
        _ if is_module_binding && facts.witness_shared_module_binding => CaptureKind::Shared,
        _ if is_module_binding => CaptureKind::Shared,
        _ => CaptureKind::Immutable,
    };

    let access = match kind {
        CaptureKind::Shared => CaptureAccess::SharedCell,
        CaptureKind::OwnedMutable => {
            // Every arm that yields OwnedMutable requires `is_local`, so the
            // pre-fusion guard `kind == OwnedMutable && resolve_local(..).is_some()`
            // on the owned-mutable body map is exactly this arm.
            debug_assert!(
                is_local,
                "OwnedMutable capture must have a local target: {facts:?}"
            );
            CaptureAccess::OwnedMutableCell
        }
        // The residual: cell access is needed but the kind stayed Immutable.
        CaptureKind::Immutable => CaptureAccess::MutableCell,
    };

    CapturePlan { kind, access }
}

impl BytecodeCompiler {
    /// Gather [`CaptureBindingFacts`] for one captured name, in the enclosing
    /// function's scope.
    fn capture_binding_facts(&self, name: &str, mutated: bool) -> CaptureBindingFacts {
        let target = if let Some(local_idx) = self.resolve_local(name) {
            Some(CaptureTarget::Local(local_idx))
        } else if let Some(scoped) = self.resolve_scoped_module_binding_name(name) {
            self.module_bindings
                .get(&scoped)
                .copied()
                .map(CaptureTarget::ModuleBinding)
        } else {
            self.module_bindings
                .get(name)
                .copied()
                .map(CaptureTarget::ModuleBinding)
        };

        let storage = match target {
            Some(CaptureTarget::Local(idx)) => self.mir_storage_class_for_slot(idx),
            _ => None,
        };

        CaptureBindingFacts {
            name: name.to_string(),
            target,
            ownership: self
                .binding_semantics_for_name(name)
                .map(|(_, _, sem)| sem.ownership_class),
            storage,
            mutated,
            boxed: self.boxed_locals.contains(name),
            witness_shared_local: self.shared_locals.contains(name),
            witness_shared_module_binding: self.shared_module_binding_contains(name),
            witness_owned_mutable_local: self.owned_mutable_locals.contains(name),
        }
    }

    /// THE producer. One call, one plan per capture, in `captured_vars` order.
    ///
    /// Called once per closure literal, before the closure body is compiled.
    /// Returns the facts alongside the plan so the escape veto (B0003) and the
    /// storage-promotion bookkeeping can read the same snapshot the selector
    /// saw, rather than re-deriving it from a compiler whose type tracker a
    /// nested `compile_function` may have moved on.
    pub(crate) fn plan_captures(
        &self,
        captured_vars: &[String],
        mutated_captures: &std::collections::HashSet<String>,
    ) -> Vec<(CaptureBindingFacts, CapturePlan)> {
        captured_vars
            .iter()
            .map(|name| {
                let facts = self.capture_binding_facts(name, mutated_captures.contains(name));
                let plan = infer_plan(&facts);
                (facts, plan)
            })
            .collect()
    }

    /// Build the closure's [`CapturePack`] from the plan. Stamps each
    /// capture's resolved `ConcreteType` — the same type the layout is built
    /// from — so the pack is a faithful model of the emitted artifact.
    pub(crate) fn build_capture_pack(
        &mut self,
        func_idx: u16,
        plan: &[(CaptureBindingFacts, CapturePlan)],
    ) -> CapturePack {
        let descriptors = plan
            .iter()
            .enumerate()
            .map(|(i, (facts, p))| {
                let capture_type = self.resolve_capture_concrete_type(&facts.name);
                CaptureDescriptor {
                    index: i as u16,
                    target: facts.target,
                    capture_type,
                    lowered: p.kind(),
                    access: p.access(),
                    storage: facts.storage,
                    name: facts.name.clone(),
                }
            })
            .collect();
        CapturePack {
            closure: func_idx,
            descriptors,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::BytecodeCompiler;

    // ───────────────────────────────────────────────────────────────────
    // (a) FUSION EQUIVALENCE — the fused plan reproduces the pre-fusion
    //     `(mutable_flags[i], capture_kinds[i])` pair across the FULL
    //     (tier × ownership × mutated × boxed × witness) cross-product.
    //
    // `legacy_pair` below is the pre-fusion code transcribed verbatim from
    // closures.rs:3236-3256 (mutable_flags) and :3550-3635 (capture_kinds).
    // It is the ORACLE, not a paraphrase.
    // ───────────────────────────────────────────────────────────────────

    fn legacy_pair(f: &CaptureBindingFacts) -> (bool, CaptureKind) {
        let is_local_slot = f.is_local();
        let is_module_binding_slot = f.is_module_binding();

        // closures.rs:3236-3256
        let is_flexible_capture = matches!(f.ownership, Some(BindingOwnershipClass::Flexible))
            && (is_local_slot || is_module_binding_slot);
        let mutable_flag = f.mutated
            || f.boxed
            || f.witness_shared_local
            || f.witness_shared_module_binding
            || is_flexible_capture;

        // closures.rs:3550-3635
        let kind = if !mutable_flag {
            CaptureKind::Immutable
        } else {
            match f.ownership {
                Some(BindingOwnershipClass::OwnedMutable) if is_local_slot => {
                    CaptureKind::OwnedMutable
                }
                Some(BindingOwnershipClass::OwnedMutable) if is_module_binding_slot => {
                    CaptureKind::Shared
                }
                Some(BindingOwnershipClass::OwnedMutable) => CaptureKind::Immutable,
                Some(BindingOwnershipClass::Flexible)
                    if is_local_slot || is_module_binding_slot =>
                {
                    CaptureKind::Shared
                }
                Some(BindingOwnershipClass::Flexible) => CaptureKind::Immutable,
                _ if is_local_slot && f.witness_shared_local => CaptureKind::Shared,
                _ if is_local_slot && f.witness_owned_mutable_local => CaptureKind::OwnedMutable,
                _ if is_module_binding_slot && f.witness_shared_module_binding => {
                    CaptureKind::Shared
                }
                _ if is_module_binding_slot => CaptureKind::Shared,
                _ => CaptureKind::Immutable,
            }
        };
        (mutable_flag, kind)
    }

    fn facts_for(
        target: Option<CaptureTarget>,
        ownership: Option<BindingOwnershipClass>,
        mutated: bool,
        boxed: bool,
        witness_shared_local: bool,
        witness_shared_module_binding: bool,
        witness_owned_mutable_local: bool,
    ) -> CaptureBindingFacts {
        CaptureBindingFacts {
            name: "x".to_string(),
            target,
            ownership,
            storage: None,
            mutated,
            boxed,
            witness_shared_local,
            witness_shared_module_binding,
            witness_owned_mutable_local,
        }
    }

    #[test]
    fn fused_plan_matches_legacy_pair_across_cross_product() {
        let tiers = [
            None,
            Some(CaptureTarget::Local(3)),
            Some(CaptureTarget::ModuleBinding(7)),
        ];
        let ownerships = [
            None,
            Some(BindingOwnershipClass::OwnedImmutable),
            Some(BindingOwnershipClass::OwnedMutable),
            Some(BindingOwnershipClass::Flexible),
        ];
        let bools = [false, true];

        let mut seen_param = false;
        let mut seen_owned_mutable_cell = false;
        let mut seen_shared_cell = false;
        let mut seen_mutable_cell = false;
        let mut cases = 0usize;

        for target in tiers {
            for ownership in ownerships {
                for mutated in bools {
                    for boxed in bools {
                        for wsl in bools {
                            for wsm in bools {
                                for woml in bools {
                                    let facts = facts_for(
                                        target, ownership, mutated, boxed, wsl, wsm, woml,
                                    );
                                    let (legacy_flag, legacy_kind) = legacy_pair(&facts);
                                    let plan = infer_plan(&facts);
                                    cases += 1;

                                    assert_eq!(
                                        plan.kind(),
                                        legacy_kind,
                                        "kind divergence for {facts:?}"
                                    );
                                    assert_eq!(
                                        plan.needs_cell(),
                                        legacy_flag,
                                        "mutable-flag divergence for {facts:?}"
                                    );

                                    // The access refinement must be a faithful
                                    // decomposition of the legacy pair.
                                    let expected_access = match (legacy_flag, legacy_kind) {
                                        (false, _) => CaptureAccess::Param,
                                        (true, CaptureKind::OwnedMutable) => {
                                            CaptureAccess::OwnedMutableCell
                                        }
                                        (true, CaptureKind::Shared) => CaptureAccess::SharedCell,
                                        // THE RESIDUAL: cell access needed,
                                        // kind stayed Immutable.
                                        (true, CaptureKind::Immutable) => {
                                            CaptureAccess::MutableCell
                                        }
                                    };
                                    assert_eq!(
                                        plan.access(),
                                        expected_access,
                                        "access divergence for {facts:?}"
                                    );

                                    match plan.access() {
                                        CaptureAccess::Param => seen_param = true,
                                        CaptureAccess::OwnedMutableCell => {
                                            seen_owned_mutable_cell = true
                                        }
                                        CaptureAccess::SharedCell => seen_shared_cell = true,
                                        CaptureAccess::MutableCell => seen_mutable_cell = true,
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        assert_eq!(
            cases,
            3 * 4 * 2 * 2 * 2 * 2 * 2,
            "full cross-product covered"
        );
        assert!(seen_param, "Param arm exercised");
        assert!(seen_owned_mutable_cell, "OwnedMutableCell arm exercised");
        assert!(seen_shared_cell, "SharedCell arm exercised");
        assert!(
            seen_mutable_cell,
            "the degenerate `mutable_flags==true ∧ kind==Immutable` residual MUST be \
             reachable on the inferred path — if it is not, the fusion silently dropped a \
             live emission arm"
        );
    }

    /// The residual arm, pinned by name so a future edit that "cleans it up"
    /// has to delete an assertion rather than a comment: an `OwnedImmutable`
    /// (a `let`) binding that a previous pass boxed needs cell access but the
    /// classifier still calls it `Immutable`.
    #[test]
    fn boxed_let_capture_is_the_mutable_cell_residual() {
        let facts = facts_for(
            Some(CaptureTarget::Local(1)),
            Some(BindingOwnershipClass::OwnedImmutable),
            false,
            true, // boxed
            false,
            false,
            false,
        );
        let plan = infer_plan(&facts);
        assert!(plan.needs_cell(), "boxed local needs cell access");
        assert_eq!(plan.access(), CaptureAccess::MutableCell);
        assert!(
            !plan.kind_is_cell_backed(),
            "the residual keeps the Immutable kind — the layout masks stay clear"
        );
    }

    // ───────────────────────────────────────────────────────────────────
    // (b) MODEL-vs-EMISSION — read from the EMITTED artifact
    //     (`program.closure_function_layouts[fid]`), never from the model's
    //     own table. This is the R2 assertion: if a future declared-mode path
    //     writes the pack but leaves emission on a second inference vector,
    //     THIS test fails.
    // ───────────────────────────────────────────────────────────────────

    fn compile(src: &str) -> BytecodeCompiler {
        let program = shape_ast::parse_program(src).expect("fixture parses");
        let mut compiler = BytecodeCompiler::new();
        compiler
            .compile_in_place(&program)
            .expect("fixture compiles");
        compiler
    }

    /// For every closure the compiler planned, the EMITTED `ClosureLayout`'s
    /// per-capture storage kind equals the pack's `lowered`, and the three
    /// capture masks agree with the plan bit-for-bit and stay disjoint.
    fn assert_model_equals_emission(compiler: &BytecodeCompiler) {
        assert!(
            !compiler.closure_capture_packs.is_empty(),
            "fixture must produce at least one closure pack"
        );
        for pack in &compiler.closure_capture_packs {
            let layout = compiler
                .program
                .closure_function_layouts
                .get(pack.closure as usize)
                .and_then(|l| l.as_ref())
                .unwrap_or_else(|| panic!("closure {} has no emitted layout", pack.closure));
            assert_eq!(
                layout.capture_kinds.len(),
                pack.len(),
                "closure {}: emitted capture count",
                pack.closure
            );
            for d in &pack.descriptors {
                let i = d.index as usize;
                // THE artifact read — not `pack.kinds()`.
                assert_eq!(
                    layout.capture_storage_kind(i),
                    d.lowered,
                    "closure {} capture {}: emitted kind != planned kind",
                    pack.closure,
                    i
                );
                let bit = 1u64 << i;
                let heap = layout.heap_capture_mask & bit != 0;
                let owned = layout.owned_mutable_capture_mask & bit != 0;
                let shared = layout.shared_capture_mask & bit != 0;
                assert!(
                    !(heap && owned) && !(heap && shared) && !(owned && shared),
                    "closure {} capture {}: masks overlap",
                    pack.closure,
                    i
                );
                match d.access {
                    CaptureAccess::OwnedMutableCell => {
                        assert!(
                            owned,
                            "OwnedMutableCell must set owned_mutable_capture_mask"
                        );
                        assert!(!shared && !heap);
                    }
                    CaptureAccess::SharedCell => {
                        assert!(shared, "SharedCell must set shared_capture_mask");
                        assert!(!owned && !heap);
                    }
                    // Param and MutableCell both carry the Immutable kind, so
                    // the mask is TYPE-derived: heap bit iff the capture's
                    // ConcreteType is a pointer.
                    CaptureAccess::Param | CaptureAccess::MutableCell => {
                        assert!(!owned && !shared);
                        assert_eq!(
                            heap,
                            layout.captures[i].kind
                                == shape_value::v2::struct_layout::FieldKind::Ptr,
                            "closure {} capture {}: heap mask must follow the capture TYPE",
                            pack.closure,
                            i
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn emitted_layout_matches_plan_immutable_scalar() {
        let c = compile(
            r#"
fn run() -> int {
  let base = 10
  let f = |x: int| x + base
  f(1)
}
run()
"#,
        );
        assert_model_equals_emission(&c);
        let pack = &c.closure_capture_packs[0];
        assert_eq!(pack.descriptors[0].access, CaptureAccess::Param);
    }

    #[test]
    fn emitted_layout_matches_plan_owned_mutable() {
        let c = compile(
            r#"
fn run() -> int {
  let mut total = 0
  let f = |x: int| { total = total + x
    total }
  f(1)
}
run()
"#,
        );
        assert_model_equals_emission(&c);
        let pack = &c.closure_capture_packs[0];
        assert_eq!(pack.descriptors[0].access, CaptureAccess::OwnedMutableCell);
        assert!(matches!(
            pack.descriptors[0].target,
            Some(CaptureTarget::Local(_))
        ));
    }

    #[test]
    fn emitted_layout_matches_plan_shared_local_var() {
        let c = compile(
            r#"
fn run() -> int {
  var counter = 0
  let bump = |x: int| { counter = counter + x
    counter }
  let peek = |y: int| y + counter
  bump(2) + peek(1)
}
run()
"#,
        );
        assert_model_equals_emission(&c);
        // BOTH siblings must see the same shared cell — the read-only sibling
        // is Shared too, not a snapshot.
        assert_eq!(c.closure_capture_packs.len(), 2);
        for pack in &c.closure_capture_packs {
            assert_eq!(pack.descriptors[0].access, CaptureAccess::SharedCell);
        }
    }

    #[test]
    fn emitted_layout_matches_plan_shared_module_binding() {
        let c = compile(
            r#"
var hits = 0
fn run() -> int {
  let f = |x: int| { hits = hits + x
    hits }
  f(3)
}
run()
"#,
        );
        assert_model_equals_emission(&c);
        let pack = &c.closure_capture_packs[0];
        assert_eq!(pack.descriptors[0].access, CaptureAccess::SharedCell);
        assert!(matches!(
            pack.descriptors[0].target,
            Some(CaptureTarget::ModuleBinding(_))
        ));
    }

    #[test]
    fn emitted_layout_matches_plan_heap_immutable_capture() {
        let c = compile(
            r#"
fn run() -> int {
  let xs = [1, 2, 3]
  let f = |i: int| xs[i]
  f(1)
}
run()
"#,
        );
        assert_model_equals_emission(&c);
        let pack = &c.closure_capture_packs[0];
        assert_eq!(pack.descriptors[0].access, CaptureAccess::Param);
        // heap mask follows the TYPE, not the mode.
        let layout = c.program.closure_function_layouts[pack.closure as usize]
            .as_ref()
            .unwrap();
        assert_eq!(layout.heap_capture_mask & 1, 1);
    }

    #[test]
    fn emitted_layout_matches_plan_nested_closures() {
        let c = compile(
            r#"
fn run() -> int {
  let outer = 7
  let f = |x: int| {
    let g = |y: int| y + outer
    g(x)
  }
  f(1)
}
run()
"#,
        );
        assert_model_equals_emission(&c);
        assert_eq!(c.closure_capture_packs.len(), 2);
    }

    /// R3: the pack is keyed by `func_idx`, and distinct closures get distinct
    /// keys. A `Span`-keyed table collides here the moment generated AST
    /// (which parses from offset 0) is in play — that was rejection finding (2).
    #[test]
    fn packs_are_keyed_by_func_idx_and_are_distinct_per_closure() {
        let c = compile(
            r#"
fn run() -> int {
  var counter = 0
  let bump = |x: int| { counter = counter + x
    counter }
  let peek = |y: int| y + counter
  bump(2) + peek(1)
}
run()
"#,
        );
        let keys: Vec<u16> = c.closure_capture_packs.iter().map(|p| p.closure).collect();
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(keys.len(), sorted.len(), "func_idx keys must be distinct");
    }

    // ───────────────────────────────────────────────────────────────────
    // (c) SENTINEL — ONE PRODUCER. The single most load-bearing artifact of
    //     this ticket: it turns R2 from a code-review norm into a build
    //     failure. Mirrored in scripts/check-no-dynamic.sh.
    // ───────────────────────────────────────────────────────────────────

    fn walk_rs_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk_rs_files(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }

    #[test]
    fn capture_kind_is_constructed_in_exactly_one_compiler_file() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/compiler");
        let mut files = Vec::new();
        walk_rs_files(&root, &mut files);
        assert!(!files.is_empty(), "compiler source tree must be walkable");

        let needles = [
            "CaptureKind::Immutable",
            "CaptureKind::OwnedMutable",
            "CaptureKind::Shared",
        ];
        let mut offenders: Vec<String> = Vec::new();
        for path in files {
            if path.file_name().and_then(|f| f.to_str()) == Some("capture_plan.rs") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            if needles.iter().any(|n| text.contains(n)) {
                offenders.push(path.display().to_string());
            }
        }
        assert!(
            offenders.is_empty(),
            "ADR-009 C1 K1 gate: `CaptureKind::<Variant>` may be named in exactly ONE \
             bytecode-compiler file (comptime_builtins/capture_plan.rs). A second producer \
             is how the declared capture mode gets discarded while inference stays \
             authoritative — the defect that got C1 rejected. Offenders: {offenders:?}"
        );
    }
}
