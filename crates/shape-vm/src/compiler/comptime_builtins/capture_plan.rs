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

use std::collections::HashSet;

use shape_ast::ast::{CaptureClause, CaptureMode, GeneratedNodeOrigin, Span};
use shape_ast::error::{Result, ShapeError};
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

impl CapturePack {
    /// The emitted layout's `capture_kinds` vector.
    pub(crate) fn kinds(&self) -> Vec<CaptureKind> {
        self.descriptors.iter().map(|d| d.lowered).collect()
    }

    pub(crate) fn len(&self) -> usize {
        self.descriptors.len()
    }

    /// True when any capture is cell-backed — i.e. the closure's
    /// `ClosureTypeId` must be interned KINDS-AWARE, because two closures with
    /// identical capture types but different capture kinds are different
    /// closures.
    pub(crate) fn any_cell_backed(&self) -> bool {
        self.descriptors
            .iter()
            .any(|d| !matches!(d.lowered, CaptureKind::Immutable))
    }

    /// ADR-009 C1 (slice 3) — **the ruling, mechanically enforced at the
    /// artifact boundary.**
    ///
    /// User rulings 1 + 2: *the declared word IS the emitted `CaptureKind`.*
    /// `move` lowers to `Immutable` (a `let`) or `OwnedMutable` (a `let mut`);
    /// `share` lowers to `Shared`; nothing else lowers at all. If a declared
    /// mode and the kind that reached the emitted `ClosureLayout` can ever
    /// disagree, the model is wrong — so this is checked, in release, on the
    /// live compile path, against the kinds that are about to be stamped into
    /// `program.closure_function_layouts`.
    ///
    /// It can only fire on a compiler bug (a second producer overwriting the
    /// plan between `lower_declared` and layout construction — which is
    /// precisely the failure mode that got C1 rejected). It is not a fallback
    /// arm and it is not user-reachable: every user-facing disagreement is
    /// already a named rejection inside [`lower_declared`].
    pub(crate) fn declared_kinds_agree_with_emission(
        &self,
        emitted: &[CaptureKind],
    ) -> std::result::Result<(), String> {
        for descriptor in &self.descriptors {
            let Some(mode) = descriptor.declared else {
                continue;
            };
            let Some(&kind) = emitted.get(descriptor.index as usize) else {
                return Err(format!(
                    "closure {}: capture {} ('{}') is declared `{}` but the emitted layout has no \
                     kind for it",
                    self.closure,
                    descriptor.index,
                    descriptor.name,
                    mode.spelling(),
                ));
            };
            let agrees = match mode {
                CaptureMode::Move => {
                    matches!(kind, CaptureKind::Immutable | CaptureKind::OwnedMutable)
                }
                CaptureMode::Share => matches!(kind, CaptureKind::Shared),
                // A borrow never lowers — `lower_declared` rejects it.
                CaptureMode::SharedBorrow | CaptureMode::ExclusiveBorrow => false,
            };
            if !agrees {
                return Err(format!(
                    "closure {}: capture {} ('{}') was declared `{}` but the emitted layout says \
                     {:?} — a declared mode and its emitted CaptureKind may never disagree \
                     (ADR-009 C1, user rulings 1 + 2)",
                    self.closure,
                    descriptor.index,
                    descriptor.name,
                    mode.spelling(),
                    kind,
                ));
            }
        }
        Ok(())
    }

    /// Provenance tail for a capture diagnostic raised on THIS closure. Empty
    /// for an ordinary source closure, so existing source-facing diagnostics are
    /// byte-identical; inside a generated body the error names the owning
    /// declaration and the structural node path instead of pointing at
    /// handler-emitted snippet offsets.
    pub(crate) fn generated_note(&self) -> String {
        match &self.origin {
            None => String::new(),
            Some(origin) => format!(
                " (in generated function '{}', node {})",
                origin.owner_display(),
                origin.render_path()
            ),
        }
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

/// THE OTHER HALF OF THE SELECTOR — the declared path.
///
/// Total-or-reject: every `(mode × facts)` pair either yields a [`CapturePlan`]
/// or a named rejection. There is no fallback arm, no `Unknown` kind, and
/// [`CaptureAccess::MutableCell`] — the inference residual — is unreachable
/// here by construction (it is not produced by any arm below).
///
/// `mutated` is deliberately NOT an input. That is what makes the declaration
/// *drive*: `move hits` over a `let mut hits` lowers to `OwnedMutable` even
/// when the closure body only READS `hits`, where inference would pick
/// `Immutable`. This is the one place a declared branch and an inferred branch
/// observably differ, and it is the whole point of the ticket.
///
/// Rulings 1 + 2: the declared word IS the emitted kind.
///   * `move` × local `let`      → `Immutable`
///   * `move` × local `let mut`  → `OwnedMutable`
///   * `move` × module binding   → `[C0906]`
///   * `share` × shared-ownership → `Shared`
///   * `share` × plain local     → `[C0908]`
///   * `&` / `&mut`              → `[C0902]`
pub(crate) fn lower_declared(
    mode: CaptureMode,
    facts: &CaptureBindingFacts,
) -> std::result::Result<CapturePlan, String> {
    let name = &facts.name;

    // `&x` / `&mut x` — a TOTAL rejection. The spelling parses so that the
    // diagnostic can be a sentence about regions rather than a syntax error;
    // it never lowers. (R6: no region story ⇒ no borrow mode.)
    if mode.is_borrow() {
        return Err(format!(
            "[C0902] ReferenceEscapeIntoClosure: declared capture '{spelling} {name}' borrows \
             '{name}' across a closure boundary; Shape has no region story for a reference that \
             escapes into a closure — declare `move {name}` to take the value, or `share {name}` \
             to take a share of a shared-ownership cell",
            spelling = mode.spelling(),
        ));
    }

    // The capture must resolve to a compiler-issued slot. No `Immutable`
    // fallback, no `MutableCell` — an unresolvable declaration is an error.
    let Some(target) = facts.target else {
        return Err(format!(
            "[C0905] declared capture '{} {name}' does not resolve to a binding in the enclosing \
             scope",
            mode.spelling(),
        ));
    };
    let is_module_binding = matches!(target, CaptureTarget::ModuleBinding(_));

    // Shared-ownership detection. A binding is shared-ownership if it is a
    // `var` (`Flexible`), or if a SIBLING closure already promoted it to a
    // `SharedCell` (the witness sets) — the aliasing invariant (C-3/C-4): a
    // declaration may not un-share what is already shared.
    let is_flexible = matches!(facts.ownership, Some(BindingOwnershipClass::Flexible));
    let sibling_shared = facts.witness_shared_local || facts.witness_shared_module_binding;
    let is_shared_ownership = is_flexible || sibling_shared || is_module_binding;

    match mode {
        CaptureMode::SharedBorrow | CaptureMode::ExclusiveBorrow => {
            unreachable!("borrow modes rejected above")
        }

        // ── `move` ────────────────────────────────────────────────────────
        CaptureMode::Move => {
            // RULING 1 — `move` never lies. A module-level binding lives for
            // the program; there is nothing to move out of. Inference lowers
            // this to `Shared` today; the DECLARED path refuses, because a
            // declared `move` that emitted `Shared` would be a mode whose word
            // differs from its kind.
            if is_module_binding {
                return Err(format!(
                    "[C0906] module-level binding '{name}' cannot be moved into a closure; \
                     module bindings live for the program and admit no move"
                ));
            }
            // Aliasing invariant (C-3/C-4). A local `var`, or a local a sibling
            // closure already promoted to a `SharedCell`, is shared-ownership:
            // moving it would give this closure a private snapshot while the
            // sibling keeps writing the cell. Named rejection, not a silent
            // re-lowering to `Shared` (that is the `declared != lowered` gap
            // ruling 2 abolished).
            if is_flexible || sibling_shared {
                return Err(format!(
                    "[C0904] '{name}' is a shared-ownership binding and cannot be un-shared by a \
                     declared `move`; use `share {name}`"
                ));
            }
            match facts.ownership {
                // `let` / a param: snapshot by value into a leading closure
                // param. The heap mask (if the type is a pointer) is derived
                // from the TYPE by the layout, not from the mode.
                Some(BindingOwnershipClass::OwnedImmutable) => Ok(CapturePlan {
                    kind: CaptureKind::Immutable,
                    access: CaptureAccess::Param,
                }),
                // `let mut`: the unique owner moves into the closure's Box cell.
                // Reached EVEN WHEN THE BODY ONLY READS IT — the declaration,
                // not the body, decides. The source local is then poisoned
                // (`captured_let_mut_moved`), so a later outer read is a
                // use-after-move error.
                Some(BindingOwnershipClass::OwnedMutable) => Ok(CapturePlan {
                    kind: CaptureKind::OwnedMutable,
                    access: CaptureAccess::OwnedMutableCell,
                }),
                // Flexible handled above; None = the type tracker could not
                // classify the binding. REJECT — there is no `Immutable`
                // fallback on the declared path (that is how a declaration
                // gets silently downgraded).
                Some(BindingOwnershipClass::Flexible) | None => Err(format!(
                    "[C0905] declared capture 'move {name}' cannot be lowered: the ownership \
                     class of '{name}' is not known at the capture site"
                )),
            }
        }

        // ── `share` (RULING 2) ────────────────────────────────────────────
        CaptureMode::Share => {
            if !is_shared_ownership {
                return Err(format!(
                    "[C0908] '{name}' is not a shared-ownership binding; use `move {name}` \
                     (declare `var {name}` if the closure and its enclosing scope must observe \
                     the same mutable cell)"
                ));
            }
            // `var` (local or module), a sibling-promoted `SharedCell`, or a
            // module binding: one word, one kind.
            Ok(CapturePlan {
                kind: CaptureKind::Shared,
                access: CaptureAccess::SharedCell,
            })
        }
    }
}

/// One capture's planned outcome: the facts the selector saw, the plan it
/// produced, and — when a clause drove it — the declared mode.
#[derive(Debug, Clone)]
pub(crate) struct PlannedCapture {
    pub(crate) facts: CaptureBindingFacts,
    pub(crate) plan: CapturePlan,
    pub(crate) declared: Option<CaptureMode>,
}

impl BytecodeCompiler {
    /// Gather [`CaptureBindingFacts`] for one captured name, in the enclosing
    /// function's scope.
    fn capture_binding_facts(&self, name: &str, mutated: bool) -> CaptureBindingFacts {
        // ONE slot resolver, shared with the declared-clause set diff — so a
        // declared entry and the discovered capture it describes can never
        // resolve to different targets.
        let target = self.resolve_capture_target(name);

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
    ///
    /// ADR-009 C1 (slice 3) — ONE SELECTOR, TWO SOURCES OF TRUTH FOR *KIND*,
    /// never both at once:
    ///
    ///   * `declared == None` → [`infer_plan`] per capture (ordinary source).
    ///   * `declared == Some(clause)` → the clause is validated against
    ///     discovery (a SET DIFF OVER `CaptureTarget`, never over names — see
    ///     [`Self::validate_declared_clause`]) and then [`lower_declared`]
    ///     produces every plan. Inference does not get a vote on the kind; if
    ///     it did, `capture(x)` and no-clause would emit identical bytecode,
    ///     which is exactly the defect that got this ticket rejected once.
    pub(crate) fn plan_captures(
        &self,
        captured_vars: &[String],
        mutated_captures: &std::collections::HashSet<String>,
        declared: Option<&CaptureClause>,
        origin: Option<&GeneratedNodeOrigin>,
        closure_span: Span,
    ) -> Result<Vec<PlannedCapture>> {
        let facts: Vec<CaptureBindingFacts> = captured_vars
            .iter()
            .map(|name| self.capture_binding_facts(name, mutated_captures.contains(name)))
            .collect();

        let Some(clause) = declared else {
            return Ok(facts
                .into_iter()
                .map(|facts| {
                    let plan = infer_plan(&facts);
                    PlannedCapture {
                        facts,
                        plan,
                        declared: None,
                    }
                })
                .collect());
        };

        // Validate first — a clause that does not describe the discovered
        // capture set is an error BEFORE any lowering runs.
        let entry_for_target = self.validate_declared_clause(clause, &facts, origin, closure_span)?;

        facts
            .into_iter()
            .map(|facts| {
                // `validate_declared_clause` proved every discovered capture
                // resolves and has a matching entry; both unwraps are its
                // post-conditions.
                let target = facts.target.expect("validated: every capture resolves");
                let mode = *entry_for_target
                    .get(&target)
                    .expect("validated: every capture is declared");
                let plan = lower_declared(mode, &facts).map_err(|message| {
                    ShapeError::SemanticError {
                        message,
                        location: Some(self.span_to_source_location(closure_span)),
                    }
                })?;
                debug_assert_ne!(
                    plan.access(),
                    CaptureAccess::MutableCell,
                    "the inference residual is unreachable on the declared path"
                );
                Ok(PlannedCapture {
                    facts,
                    plan,
                    declared: Some(mode),
                })
            })
            .collect()
    }

    /// Resolve every clause entry to a compiler-issued [`CaptureTarget`] and
    /// diff the declared set against the discovered set.
    ///
    /// **The diff is over TARGETS, never over names.** This is the single
    /// easiest place for the design to rot: `EnvironmentAnalyzer` hands back
    /// `Vec<String>`, and a name-keyed comparison would silently mis-pair a
    /// shadowed binding. Both sets are mapped through the same slot resolver
    /// before they are compared.
    fn validate_declared_clause(
        &self,
        clause: &CaptureClause,
        facts: &[CaptureBindingFacts],
        origin: Option<&GeneratedNodeOrigin>,
        closure_span: Span,
    ) -> Result<std::collections::HashMap<CaptureTarget, CaptureMode>> {
        let reject = |message: String| ShapeError::SemanticError {
            message,
            location: Some(self.span_to_source_location(closure_span)),
        };

        // (i) every DECLARED entry resolves to a slot — [C0905].
        // (ii) no two entries name the same slot — [C0907].
        let mut entry_for_target: std::collections::HashMap<CaptureTarget, CaptureMode> =
            std::collections::HashMap::new();
        let mut declared_names: std::collections::HashMap<CaptureTarget, String> =
            std::collections::HashMap::new();
        for entry in &clause.entries {
            let Some(target) = self.resolve_capture_target(&entry.name) else {
                return Err(reject(format!(
                    "[C0905] declared capture '{} {}' does not resolve to a binding in the \
                     enclosing scope",
                    entry.mode.spelling(),
                    entry.name,
                )));
            };
            if let Some(previous) = declared_names.insert(target, entry.name.clone()) {
                return Err(reject(format!(
                    "[C0907] duplicate capture declaration for '{}'{}; each captured binding may \
                     be declared exactly once",
                    entry.name,
                    if previous == entry.name {
                        String::new()
                    } else {
                        format!(" (already declared as '{previous}')")
                    },
                )));
            }
            entry_for_target.insert(target, entry.mode);
        }

        // (iii) every DISCOVERED capture resolves to a slot — [C0905]. Slice 1
        //       left `CaptureBindingFacts.target` an `Option` because the
        //       pre-fusion selector had live `None` arms; the declared path
        //       rejects `None` by name rather than inheriting an `Immutable`
        //       fallback.
        let mut discovered: HashSet<CaptureTarget> = HashSet::new();
        for f in facts {
            let Some(target) = f.target else {
                return Err(reject(format!(
                    "[C0905] captured binding '{}' does not resolve to a frame local or a module \
                     binding; it cannot be declared",
                    f.name,
                )));
            };
            discovered.insert(target);
        }

        // (iv) THE SET DIFF, both directions.
        //
        // declared ∖ discovered — a declaration for something the body never
        // reads. Not a warning: a stale declaration is how a generated closure
        // silently keeps a capture alive after the body that used it changed.
        let mut unused: Vec<&str> = clause
            .entries
            .iter()
            .filter(|entry| {
                self.resolve_capture_target(&entry.name)
                    .is_none_or(|target| !discovered.contains(&target))
            })
            .map(|entry| entry.name.as_str())
            .collect();
        unused.sort_unstable();
        unused.dedup();
        if !unused.is_empty() {
            let names = unused
                .iter()
                .map(|n| format!("'{n}'"))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(reject(format!(
                "[C0901] declared capture {names} is never used by the closure body; remove the \
                 declaration"
            )));
        }

        // discovered ∖ declared — the Wave-46 used-but-undeclared error. The
        // message is the EXISTING implicit-capture message, verbatim: an
        // undeclared capture inside generated code IS an implicit capture,
        // whether the closure carried a partial clause or no clause at all.
        let mut undeclared: Vec<&str> = facts
            .iter()
            .filter(|f| {
                f.target
                    .is_none_or(|target| !entry_for_target.contains_key(&target))
            })
            .map(|f| f.name.as_str())
            .collect();
        undeclared.sort_unstable();
        if !undeclared.is_empty() {
            return Err(reject(implicit_capture_message(&undeclared, origin)));
        }

        Ok(entry_for_target)
    }

    /// The slot resolver both halves of the set diff run through.
    pub(crate) fn resolve_capture_target(&self, name: &str) -> Option<CaptureTarget> {
        if let Some(local_idx) = self.resolve_local(name) {
            return Some(CaptureTarget::Local(local_idx));
        }
        if let Some(scoped) = self.resolve_scoped_module_binding_name(name)
            && let Some(&idx) = self.module_bindings.get(&scoped)
        {
            return Some(CaptureTarget::ModuleBinding(idx));
        }
        self.module_bindings
            .get(name)
            .copied()
            .map(CaptureTarget::ModuleBinding)
    }

    /// THE `ClosureTypeId` producer — used by BOTH emission
    /// (`compile_expr_closure`) and the monomorphization pre-pass
    /// (`mint_closure_type_id_peek`).
    ///
    /// The id is the closure's layout identity. When every capture is a
    /// snapshot param the types-only intern is canonical; as soon as any
    /// capture is cell-backed the kinds must enter the key, or two closures
    /// with identical capture TYPES but different capture KINDS collide.
    ///
    /// Both call sites route through here so the id the mono cache is keyed on
    /// is the id the emitted closure carries. Before slice 3 the peek was
    /// unconditionally types-only, so it diverged from emission for every
    /// cell-backed capture; the DECLARED path makes that divergence load-bearing
    /// (`move hits` over a read-only `let mut` is precisely the case where
    /// inference says all-Immutable and the declaration says `OwnedMutable`).
    pub(crate) fn intern_closure_type_id_for_pack(
        &mut self,
        pack: &CapturePack,
    ) -> shape_value::v2::concrete_type::ClosureTypeId {
        // The pack's `capture_type`s are THE types the emitted `ClosureLayout`
        // is built from (`compiler_impl_reference_model.rs`), so the interned
        // id and the layout can never be keyed on different types.
        let capture_types: Vec<ConcreteType> = pack
            .descriptors
            .iter()
            .map(|d| d.capture_type.clone())
            .collect();
        if pack.any_cell_backed() {
            self.closure_registry
                .intern_with_kinds(capture_types, pack.kinds())
        } else {
            self.closure_registry.intern(capture_types)
        }
    }

    /// Build the closure's [`CapturePack`] from the plan. Stamps each
    /// capture's resolved `ConcreteType` — the same type the layout is built
    /// from — so the pack is a faithful model of the emitted artifact.
    pub(crate) fn build_capture_pack(
        &mut self,
        func_idx: u16,
        plan: &[PlannedCapture],
        origin: Option<&GeneratedNodeOrigin>,
    ) -> CapturePack {
        let descriptors = plan
            .iter()
            .enumerate()
            .map(|(i, planned)| {
                let capture_type = self.resolve_capture_concrete_type(&planned.facts.name);
                CaptureDescriptor {
                    index: i as u16,
                    target: planned.facts.target,
                    capture_type,
                    declared: planned.declared,
                    lowered: planned.plan.kind(),
                    access: planned.plan.access(),
                    storage: planned.facts.storage,
                    name: planned.facts.name.clone(),
                }
            })
            .collect();
        CapturePack {
            closure: func_idx,
            origin: origin.cloned(),
            descriptors,
        }
    }
}

/// The Wave-46 implicit-capture rejection — ONE producer, so the message the
/// no-clause path raises and the message the used-but-undeclared diff raises
/// are the same sentence by construction.
///
/// `origin` is `Some` on the no-clause path (where the closure's own span points
/// at handler-emitted snippet offsets that resolve nowhere in the user's file,
/// so the error must name the owning expansion instead).
pub(crate) fn implicit_capture_message(
    names: &[&str],
    origin: Option<&GeneratedNodeOrigin>,
) -> String {
    let captures = names
        .iter()
        .map(|name| format!("'{name}'"))
        .collect::<Vec<_>>()
        .join(", ");
    match origin {
        Some(origin) => format!(
            "generated closure implicitly captures {captures}; generated captures must be \
             explicit (in generated function '{}', node {})",
            origin.owner_display(),
            origin.render_path()
        ),
        None => format!(
            "generated closure implicitly captures {captures}; generated captures must be explicit"
        ),
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
        assert_eq!(
            plan.kind(),
            CaptureKind::Immutable,
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

    pub(super) fn compile(src: &str) -> BytecodeCompiler {
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


// ═══════════════════════════════════════════════════════════════════════════
// ADR-009 C1 SLICE 3 — THE DECLARED PATH.
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod declared_tests {
    use super::tests::compile;
    use super::*;
    use crate::compiler::BytecodeCompiler;
    use shape_ast::ast::CaptureMode;

    // ───────────────────────────────────────────────────────────────────
    // (1) THE DISTINGUISHING ACCEPT PAIR — the test the rejected C1 could
    //     not write.
    //
    // `let mut hits`, READ-ONLY in the closure body. Inference gives
    // `Immutable` + a leading param (the `!mutable_flags[i]` short-circuit).
    // The DECLARATION says `move`, and `move` × `let mut` is `OwnedMutable`.
    //
    // SELF-TEST (the reason C1's accept test was worthless): DELETE the
    // clause from `flagship_declared` and the assertions below FAIL — the
    // capture reverts to `Immutable`/`Param`. The declaration is doing the
    // work, not inference. `flagship_inferred_source` is that deletion,
    // pinned as the negative control.
    // ───────────────────────────────────────────────────────────────────

    /// A generated `extend Job { method scale }` — THE FLAGSHIP surface — whose
    /// closure declares `move hits` over a `let mut` it only READS.
    const FLAGSHIP_DECLARED: &str = r#"
annotation add_scaler() {
  targets: [type]
  comptime post(target, ctx) {
    extend (f"extend {target.name} \{ method scale(f: int) -> int \{ let mut hits = 3
      let worker = |x: int; move hits| x * hits
      worker(f) \} \}")
  }
}

@add_scaler()
type Job { id: int }

let job = Job { id: 1 }
job.scale(2)
"#;

    /// THE NEGATIVE CONTROL: byte-for-byte the same closure body and the same
    /// `let mut hits`, with NO clause, in ORDINARY SOURCE (the clause is a
    /// generated-code-only surface, so the control cannot also be generated).
    /// Inference picks `Immutable`.
    const FLAGSHIP_INFERRED_SOURCE: &str = r#"
fn scale(f: int) -> int {
  let mut hits = 3
  let worker = |x: int| x * hits
  worker(f)
}
scale(2)
"#;

    /// The closure function's own instruction window. Reading the OPCODES is
    /// the half of the accept proof a layout-mask assertion cannot give you:
    /// the rejected C1 branch set the mask correctly and still emitted a
    /// leading-param load in the body.
    fn closure_body_opcodes(c: &BytecodeCompiler, func_idx: u16) -> Vec<crate::bytecode::OpCode> {
        let function = &c.program.functions[func_idx as usize];
        let start = function.entry_point;
        let end = (start + function.body_length).min(c.program.instructions.len());
        c.program.instructions[start..end]
            .iter()
            .map(|instruction| instruction.opcode)
            .collect()
    }

    fn sole_pack(c: &BytecodeCompiler) -> &CapturePack {
        let packs: Vec<&CapturePack> = c
            .closure_capture_packs
            .iter()
            .filter(|p| p.len() == 1 && p.descriptors[0].name == "hits")
            .collect();
        assert_eq!(
            packs.len(),
            1,
            "fixture must produce exactly one `hits`-capturing closure"
        );
        packs[0]
    }

    /// Read the EMITTED artifact — `program.closure_function_layouts[fid]` —
    /// never the model's own table (R2).
    fn emitted(
        c: &BytecodeCompiler,
        pack: &CapturePack,
    ) -> std::sync::Arc<shape_value::v2::closure_layout::ClosureLayout> {
        c.program.closure_function_layouts[pack.closure as usize]
            .as_ref()
            .expect("closure has an emitted layout")
            .clone()
    }

    #[test]
    fn flagship_declared_move_over_read_only_let_mut_emits_owned_mutable() {
        let c = compile(FLAGSHIP_DECLARED);
        let pack = sole_pack(&c);
        let layout = emitted(&c, pack);

        // THE assertion: the EMITTED layout, not the plan.
        assert_eq!(
            layout.capture_storage_kind(0),
            CaptureKind::OwnedMutable,
            "the declared `move` over a `let mut` must reach the emitted ClosureLayout"
        );
        assert_eq!(
            layout.owned_mutable_capture_mask & 1,
            1,
            "owned_mutable_capture_mask bit must be set"
        );
        assert_eq!(layout.shared_capture_mask & 1, 0);
        // `hits: int` — the heap mask follows the TYPE, not the mode.
        assert_eq!(layout.heap_capture_mask & 1, 0);

        // The BODY must reach the capture through the owned-mutable cell, not
        // as a leading immutable param. This is the half C1 got wrong: it
        // flipped the layout mask while emission still read a param.
        let closure_fn = &c.program.functions[pack.closure as usize];
        assert_eq!(
            closure_fn.mutable_captures,
            vec![true],
            "`Function.mutable_captures` must say the body reads a cell"
        );
        assert_eq!(pack.descriptors[0].access, CaptureAccess::OwnedMutableCell);
        assert_eq!(pack.descriptors[0].declared, Some(CaptureMode::Move));

        // The owned-mutable capture OPCODES, not a `LoadLocal` of a leading
        // param slot. `capture_storage_kind` alone cannot prove this — the
        // rejected C1 branch passed a mask assertion and still emitted a param
        // read.
        let body = closure_body_opcodes(&c, pack.closure);
        assert!(
            body.iter().any(|op| format!("{op:?}").contains("OwnedMutable")),
            "closure body must emit Load/StoreOwnedMutableCapture; got {body:?}"
        );
    }

    /// SELF-TEST, executed: the same program WITHOUT the declaration (in
    /// ordinary source, where inference is legal) emits `Immutable` + a leading
    /// param. If this test and the one above ever agree, the declaration is
    /// being discarded — which is exactly rejection finding (1).
    #[test]
    fn negative_control_no_clause_infers_immutable_param() {
        let c = compile(FLAGSHIP_INFERRED_SOURCE);
        let pack = sole_pack(&c);
        let layout = emitted(&c, pack);

        assert_eq!(
            layout.capture_storage_kind(0),
            CaptureKind::Immutable,
            "inference over a READ-ONLY `let mut` picks Immutable — this is the \
             behaviour the declaration must be able to override"
        );
        assert_eq!(layout.owned_mutable_capture_mask & 1, 0);
        assert_eq!(pack.descriptors[0].access, CaptureAccess::Param);
        assert_eq!(pack.descriptors[0].declared, None);
        assert_eq!(
            c.program.functions[pack.closure as usize].mutable_captures,
            vec![false]
        );

        let body = closure_body_opcodes(&c, pack.closure);
        assert!(
            !body.iter().any(|op| format!("{op:?}").contains("OwnedMutable")),
            "the inferred closure must NOT emit owned-mutable capture opcodes; got {body:?}"
        );
    }

    /// The two halves of the pair, side by side, in ONE assertion — so a future
    /// edit cannot make them agree without deleting this line.
    #[test]
    fn declaration_changes_the_emitted_bytecode() {
        let declared = compile(FLAGSHIP_DECLARED);
        let inferred = compile(FLAGSHIP_INFERRED_SOURCE);
        let dk = emitted(&declared, sole_pack(&declared)).capture_storage_kind(0);
        let ik = emitted(&inferred, sole_pack(&inferred)).capture_storage_kind(0);
        assert_ne!(
            dk, ik,
            "a declared capture mode MUST produce different bytecode from inference; \
             identical output is the defect that got C1 rejected"
        );
        assert_eq!(dk, CaptureKind::OwnedMutable);
        assert_eq!(ik, CaptureKind::Immutable);
    }

    /// `share` over a `var` (ruling 2) — the declared word IS the kind. The
    /// `shared_capture_mask` bit is set on the EMITTED layout.
    ///
    /// The binding is a LOCAL `var`, not a module-level one, for a reason that
    /// is worth writing down: a generated `extend` method cannot reference a
    /// module-level binding AT ALL on main — `extend Job { method read(x: int)
    /// { x + total } }` fails with "Undefined variable: 'total'" with no closure
    /// anywhere in sight. That is a pre-existing scoping limitation of generated
    /// extend bodies, not something the declared-capture path introduces, and it
    /// is out of this ticket's territory. The module-binding arms of the ruling
    /// (`share` → Shared, `move` → [C0906]) are pinned at the `lower_declared`
    /// level instead, where the facts can be stated directly.
    #[test]
    fn declared_share_over_local_var_emits_shared() {
        let c = compile(
            r#"
annotation add_reader() {
  targets: [type]
  comptime post(target, ctx) {
    extend (f"extend {target.name} \{ method read(x: int) -> int \{ var total = 5
      let worker = |y: int; share total| y + total
      worker(x) \} \}")
  }
}

@add_reader()
type Job { id: int }

let job = Job { id: 1 }
job.read(2)
"#,
        );
        let pack = c
            .closure_capture_packs
            .iter()
            .find(|p| p.descriptors.iter().any(|d| d.name == "total"))
            .expect("the `total`-capturing closure");
        let layout = emitted(&c, pack);
        assert_eq!(layout.capture_storage_kind(0), CaptureKind::Shared);
        assert_eq!(layout.shared_capture_mask & 1, 1);
        assert_eq!(layout.owned_mutable_capture_mask & 1, 0);
        assert_eq!(pack.descriptors[0].declared, Some(CaptureMode::Share));
        assert_eq!(pack.descriptors[0].access, CaptureAccess::SharedCell);
        assert!(matches!(
            pack.descriptors[0].target,
            Some(CaptureTarget::Local(_))
        ));

        // The body must reach it through the SHARED cell opcodes.
        let body = closure_body_opcodes(&c, pack.closure);
        assert!(
            body.iter()
                .any(|op| format!("{op:?}").contains("SharedCapture")),
            "closure body must emit Load/StoreSharedCapture; got {body:?}"
        );
    }

    /// `move` over a `let` — `Immutable`, and when the value is a heap type the
    /// `heap_capture_mask` bit follows the TYPE, not the mode.
    #[test]
    fn declared_move_over_let_string_emits_immutable_with_heap_mask() {
        let c = compile(
            r#"
annotation add_reader() {
  targets: [type]
  comptime post(target, ctx) {
    extend ("extend Job { method read() -> string { let tag = \"hi\"
      let worker = |; move tag| tag
      worker() } }")
  }
}

@add_reader()
type Job { id: int }

let job = Job { id: 1 }
job.read()
"#,
        );
        let pack = c
            .closure_capture_packs
            .iter()
            .find(|p| p.descriptors.iter().any(|d| d.name == "tag"))
            .expect("the `tag`-capturing closure");
        let layout = emitted(&c, pack);
        assert_eq!(layout.capture_storage_kind(0), CaptureKind::Immutable);
        assert_eq!(layout.owned_mutable_capture_mask & 1, 0);
        assert_eq!(layout.shared_capture_mask & 1, 0);
        assert_eq!(
            layout.heap_capture_mask & 1,
            1,
            "a `string` capture is heap-refcounted regardless of the declared mode"
        );
        assert_eq!(pack.descriptors[0].declared, Some(CaptureMode::Move));
    }

    /// R3: the pack of a MONOMORPHIZED generated body is keyed by its own
    /// `func_idx` and carries the declared mode into the specialization. A
    /// span-keyed table collides here (generated AST parses from offset 0).
    #[test]
    fn declared_mode_survives_monomorphization() {
        let c = compile(
            r#"
annotation add_scaler() {
  targets: [type]
  comptime post(target, ctx) {
    extend (f"extend {target.name} \{ method scale<T>(f: T) -> int \{ let mut hits = 3
      let worker = |x: T; move hits| hits
      worker(f) \} \}")
  }
}

@add_scaler()
type Job { id: int }

let job = Job { id: 1 }
job.scale(2)
"#,
        );
        let packs: Vec<&CapturePack> = c
            .closure_capture_packs
            .iter()
            .filter(|p| p.descriptors.iter().any(|d| d.name == "hits"))
            .collect();
        assert!(
            !packs.is_empty(),
            "the specialization must produce a `hits` pack"
        );
        for pack in packs {
            assert_eq!(pack.descriptors[0].declared, Some(CaptureMode::Move));
            assert_eq!(
                emitted(&c, pack).capture_storage_kind(0),
                CaptureKind::OwnedMutable,
                "the declaration must drive the SPECIALIZATION's layout too"
            );
        }
    }

    // ───────────────────────────────────────────────────────────────────
    // (3) THE REJECTION MATRIX — `lower_declared` is TOTAL-OR-REJECT.
    //     Every `(mode × facts)` pair below either lowers or names a code.
    // ───────────────────────────────────────────────────────────────────

    fn facts(
        target: Option<CaptureTarget>,
        ownership: Option<BindingOwnershipClass>,
        witness_shared_local: bool,
    ) -> CaptureBindingFacts {
        CaptureBindingFacts {
            name: "x".to_string(),
            target,
            ownership,
            storage: None,
            mutated: false,
            boxed: false,
            witness_shared_local,
            witness_shared_module_binding: false,
            witness_owned_mutable_local: false,
        }
    }

    fn reject(mode: CaptureMode, f: &CaptureBindingFacts) -> String {
        lower_declared(mode, f).expect_err("must reject")
    }

    #[test]
    fn c0902_borrow_modes_are_a_total_rejection() {
        for mode in [CaptureMode::SharedBorrow, CaptureMode::ExclusiveBorrow] {
            // Every fact shape — a borrow never lowers, whatever it points at.
            for target in [
                None,
                Some(CaptureTarget::Local(1)),
                Some(CaptureTarget::ModuleBinding(1)),
            ] {
                for ownership in [
                    None,
                    Some(BindingOwnershipClass::OwnedImmutable),
                    Some(BindingOwnershipClass::OwnedMutable),
                    Some(BindingOwnershipClass::Flexible),
                ] {
                    let message = reject(mode, &facts(target, ownership, false));
                    assert!(
                        message.starts_with("[C0902] ReferenceEscapeIntoClosure:"),
                        "got {message}"
                    );
                }
            }
        }
    }

    #[test]
    fn c0905_unresolvable_target_is_rejected_not_defaulted() {
        for mode in [CaptureMode::Move, CaptureMode::Share] {
            let message = reject(mode, &facts(None, Some(BindingOwnershipClass::OwnedImmutable), false));
            assert!(message.starts_with("[C0905]"), "got {message}");
        }
    }

    /// [C0905] — the `move` × unknown-ownership arm. NO `Immutable` fallback,
    /// no `MutableCell`: an ownership class the compiler cannot name is an
    /// error, because guessing it is how a declaration gets silently
    /// downgraded.
    #[test]
    fn c0905_unknown_ownership_class_is_rejected_not_defaulted() {
        let message = reject(
            CaptureMode::Move,
            &facts(Some(CaptureTarget::Local(1)), None, false),
        );
        assert!(message.starts_with("[C0905]"), "got {message}");
    }

    /// RULING 1 — `move` never lies. A module-level binding admits no move.
    /// Inference lowers exactly this shape to `Shared`; the declared path
    /// refuses rather than emit a kind whose name is not the declared word.
    #[test]
    fn c0906_move_on_module_binding_is_rejected() {
        for ownership in [
            Some(BindingOwnershipClass::OwnedImmutable),
            Some(BindingOwnershipClass::OwnedMutable),
            Some(BindingOwnershipClass::Flexible),
        ] {
            let message = reject(
                CaptureMode::Move,
                &facts(Some(CaptureTarget::ModuleBinding(2)), ownership, false),
            );
            assert_eq!(
                message,
                "[C0906] module-level binding 'x' cannot be moved into a closure; module \
                 bindings live for the program and admit no move"
            );
        }
    }

    /// [C0904] — a declaration may not UN-SHARE. A local `var`, or a local a
    /// sibling closure already promoted to a `SharedCell`, is shared ownership;
    /// `move` would hand this closure a private snapshot while the sibling keeps
    /// writing the cell.
    #[test]
    fn c0904_move_cannot_unshare_a_var_or_a_sibling_shared_local() {
        let var_local = reject(
            CaptureMode::Move,
            &facts(
                Some(CaptureTarget::Local(1)),
                Some(BindingOwnershipClass::Flexible),
                false,
            ),
        );
        assert!(var_local.starts_with("[C0904]"), "got {var_local}");

        let sibling_shared = reject(
            CaptureMode::Move,
            &facts(
                Some(CaptureTarget::Local(1)),
                Some(BindingOwnershipClass::OwnedImmutable),
                true, // a sibling closure already promoted it
            ),
        );
        assert!(sibling_shared.starts_with("[C0904]"), "got {sibling_shared}");
    }

    /// [C0908] (ruling 2) — `share` over a plain local: there is nothing shared
    /// to take a share OF.
    #[test]
    fn c0908_share_on_a_plain_local_is_rejected() {
        for ownership in [
            BindingOwnershipClass::OwnedImmutable,
            BindingOwnershipClass::OwnedMutable,
        ] {
            let message = reject(
                CaptureMode::Share,
                &facts(Some(CaptureTarget::Local(1)), Some(ownership), false),
            );
            assert!(message.starts_with("[C0908]"), "got {message}");
        }
    }

    /// THE ACCEPT TABLE, total. Every pair that lowers, and what it lowers to —
    /// declared word == emitted kind, with no exceptions (rulings 1 + 2).
    #[test]
    fn lower_declared_accept_table_is_the_ruling() {
        // move × local `let` → Immutable + a leading param.
        let p = lower_declared(
            CaptureMode::Move,
            &facts(
                Some(CaptureTarget::Local(1)),
                Some(BindingOwnershipClass::OwnedImmutable),
                false,
            ),
        )
        .unwrap();
        assert_eq!(p.kind(), CaptureKind::Immutable);
        assert_eq!(p.access(), CaptureAccess::Param);

        // move × local `let mut` → OwnedMutable, REGARDLESS of `mutated`.
        for mutated in [false, true] {
            let mut f = facts(
                Some(CaptureTarget::Local(1)),
                Some(BindingOwnershipClass::OwnedMutable),
                false,
            );
            f.mutated = mutated;
            let p = lower_declared(CaptureMode::Move, &f).unwrap();
            assert_eq!(
                p.kind(),
                CaptureKind::OwnedMutable,
                "the DECLARATION decides, not the body: `mutated` is not an input"
            );
            assert_eq!(p.access(), CaptureAccess::OwnedMutableCell);
        }

        // share × local `var` → Shared.
        let p = lower_declared(
            CaptureMode::Share,
            &facts(
                Some(CaptureTarget::Local(1)),
                Some(BindingOwnershipClass::Flexible),
                false,
            ),
        )
        .unwrap();
        assert_eq!(p.kind(), CaptureKind::Shared);
        assert_eq!(p.access(), CaptureAccess::SharedCell);

        // share × module binding → Shared.
        let p = lower_declared(
            CaptureMode::Share,
            &facts(
                Some(CaptureTarget::ModuleBinding(3)),
                Some(BindingOwnershipClass::OwnedMutable),
                false,
            ),
        )
        .unwrap();
        assert_eq!(p.kind(), CaptureKind::Shared);

        // share × sibling-shared local → Shared.
        let p = lower_declared(
            CaptureMode::Share,
            &facts(
                Some(CaptureTarget::Local(1)),
                Some(BindingOwnershipClass::OwnedImmutable),
                true,
            ),
        )
        .unwrap();
        assert_eq!(p.kind(), CaptureKind::Shared);
    }

    /// [B0003] — NEITHER WIDENED NOR NARROWED by a declaration.
    ///
    /// The reference-escape rule is not the declared path's to change, so this
    /// pins the declared path against the INFERRED path at the same position,
    /// in both directions:
    ///
    ///   (a) TOP-LEVEL, inferred: `[B0003]` fires. (The clause cannot reach
    ///       here — it is generated-code-only, and generated code always
    ///       compiles inside a function — so the arm is held by the inferred
    ///       path, which is the only one that can.)
    ///   (b) INSIDE A FUNCTION, inferred: it does NOT fire. The front-end arm
    ///       is guarded on `current_function.is_none()`, and the MIR solver's
    ///       `ReferenceEscapeIntoClosure` fact does not catch this shape.
    ///       That is a PRE-EXISTING hole in B0003's coverage — verified on the
    ///       INFERRED path, on the parent commit's behaviour, with no clause
    ///       anywhere in the program.
    ///   (c) INSIDE A GENERATED BODY, DECLARED `move`: identical to (b).
    ///
    /// (c) == (b) is the whole assertion. A declared `move` does not rescue a
    /// reference the compiler would otherwise reject, and it does not reject a
    /// reference the compiler would otherwise admit. If a future change makes
    /// the escape check total inside function bodies, (b) and (c) must move
    /// TOGETHER — and this test will say so, loudly, rather than letting the
    /// declared path drift ahead of or behind inference.
    #[test]
    fn b0003_is_neither_widened_nor_narrowed_by_a_declaration() {
        fn outcome(src: &str) -> std::result::Result<(), String> {
            let program = shape_ast::parse_program(src).expect("fixture parses");
            let mut compiler = BytecodeCompiler::new();
            compiler
                .compile_in_place(&program)
                .map_err(|e| e.to_string())
        }

        // (a) top level, inferred — the arm fires, VERBATIM.
        let top_level = outcome(
            r#"
let value = 7
let r = &value
let worker = |y: int| y + r
worker(2)
"#,
        )
        .expect_err("a reference cannot escape into a top-level closure");
        assert!(
            top_level.contains(
                "[B0003] reference 'r' cannot escape into a closure; capture a value instead"
            ),
            "the B0003 message must be byte-identical: {top_level}"
        );

        // (b) inside a function, inferred — the PRE-EXISTING hole.
        let inferred_in_fn = outcome(
            r#"
fn read(x: int) -> int {
  let value = 7
  let r = &value
  let worker = |y: int| y + r
  worker(x)
}
read(2)
"#,
        );

        // (c) inside a GENERATED body, DECLARED `move` — must match (b) exactly.
        let declared_in_generated = outcome(
            r#"
annotation add_reader() {
  targets: [type]
  comptime post(target, ctx) {
    extend ("extend Job { method read(x: int) -> int { let value = 7
      let r = &value
      let worker = |y: int; move r| y + r
      worker(x) } }")
  }
}
@add_reader()
type Job { id: int }
let job = Job { id: 1 }
job.read(2)
"#,
        );

        assert_eq!(
            inferred_in_fn.is_ok(),
            declared_in_generated.is_ok(),
            "a declared `move` must treat a reference capture EXACTLY as inference does \
             at the same position — inferred: {inferred_in_fn:?}, declared: \
             {declared_in_generated:?}"
        );
    }

    /// R6 / X1 — the inference residual is UNREACHABLE on the declared path.
    /// Every accepting arm of `lower_declared` is enumerated above; none of them
    /// produces `MutableCell`. This test proves it exhaustively over the fact
    /// cross-product rather than by reading the code.
    #[test]
    fn declared_path_never_produces_the_mutable_cell_residual() {
        let mut accepted = 0usize;
        for mode in [
            CaptureMode::Move,
            CaptureMode::Share,
            CaptureMode::SharedBorrow,
            CaptureMode::ExclusiveBorrow,
        ] {
            for target in [
                None,
                Some(CaptureTarget::Local(1)),
                Some(CaptureTarget::ModuleBinding(1)),
            ] {
                for ownership in [
                    None,
                    Some(BindingOwnershipClass::OwnedImmutable),
                    Some(BindingOwnershipClass::OwnedMutable),
                    Some(BindingOwnershipClass::Flexible),
                ] {
                    for mutated in [false, true] {
                        for boxed in [false, true] {
                            for wsl in [false, true] {
                                for wsm in [false, true] {
                                    let mut f = facts(target, ownership, wsl);
                                    f.mutated = mutated;
                                    f.boxed = boxed;
                                    f.witness_shared_module_binding = wsm;
                                    match lower_declared(mode, &f) {
                                        Ok(plan) => {
                                            accepted += 1;
                                            assert_ne!(
                                                plan.access(),
                                                CaptureAccess::MutableCell,
                                                "the declared path produced the inference \
                                                 residual for {f:?} / {mode:?}"
                                            );
                                            // And the ruling: word == kind.
                                            match mode {
                                                CaptureMode::Move => assert!(matches!(
                                                    plan.kind(),
                                                    CaptureKind::Immutable
                                                        | CaptureKind::OwnedMutable
                                                )),
                                                CaptureMode::Share => assert_eq!(
                                                    plan.kind(),
                                                    CaptureKind::Shared
                                                ),
                                                _ => panic!("a borrow must never lower"),
                                            }
                                        }
                                        Err(message) => assert!(
                                            message.starts_with("[C09"),
                                            "every rejection carries a code: {message}"
                                        ),
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(accepted > 0, "the accept arms must be reachable");
    }
}


