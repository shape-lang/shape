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

use crate::type_tracking::{BindingOwnershipClass, BindingStorageClass};

mod artifact;
mod model;
mod planner;
mod query;
mod surface;
mod validation;

pub(crate) use model::*;
pub use query::{
    CaptureSiteRole, GENERATED_CAPTURE_ARTIFACT_CONFLICT_CODE,
    GENERATED_CAPTURE_SOURCE_UNAVAILABLE_CODE, GeneratedCaptureBindingIdentity,
    GeneratedCaptureDescriptorView, GeneratedCaptureOccurrenceIdentity, GeneratedCaptureQuery,
    GeneratedCaptureQueryIssue, GeneratedCaptureSite, GeneratedCaptureSlot,
    GeneratedCaptureSourceMap, GeneratedCaptureSpecialization,
    GeneratedCaptureSpecializationIdentity, GeneratedCaptureStage,
};

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
    // A synthetic capture parameter whose enclosing descriptor is Shared
    // carries the raw cell identity, not an ordinary by-value parameter. This
    // structural evidence wins before the parameter's mechanically applied
    // `OwnedMutable` binding semantics; reclassifying it by those semantics is
    // the #53 descriptor-erasure bug.
    if facts.inherited_shared_cell {
        return CapturePlan::new(CaptureKind::Shared, CaptureAccess::SharedCell);
    }

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
        return CapturePlan::new(CaptureKind::Immutable, CaptureAccess::Param);
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

    CapturePlan::new(kind, access)
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
    let sibling_shared = facts.witness_shared_local
        || facts.witness_shared_module_binding
        || facts.inherited_shared_cell;
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
                Some(BindingOwnershipClass::OwnedImmutable) => Ok(CapturePlan::new(
                    CaptureKind::Immutable,
                    CaptureAccess::Param,
                )),
                // `let mut`: the unique owner moves into the closure's Box cell.
                // Reached EVEN WHEN THE BODY ONLY READS IT — the declaration,
                // not the body, decides. The source local is then poisoned
                // (`captured_let_mut_moved`), so a later outer read is a
                // use-after-move error.
                Some(BindingOwnershipClass::OwnedMutable) => Ok(CapturePlan::new(
                    CaptureKind::OwnedMutable,
                    CaptureAccess::OwnedMutableCell,
                )),
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
            Ok(CapturePlan::new(
                CaptureKind::Shared,
                CaptureAccess::SharedCell,
            ))
        }
    }
}

/// Re-derive a declared capture's exact storage kind from the retained source
/// facts at the artifact boundary. Keeping this beside the selector preserves
/// K1: no second compiler file may construct a `CaptureKind` variant.
pub(super) fn exact_declared_kind(
    descriptor: &CaptureDescriptor,
) -> std::result::Result<Option<CaptureKind>, String> {
    let Some(mode) = descriptor.declared else {
        return Ok(None);
    };
    let kind = match (mode, descriptor.target, descriptor.ownership) {
        (
            CaptureMode::Move,
            Some(CaptureTarget::Local(_)),
            Some(BindingOwnershipClass::OwnedImmutable),
        ) => CaptureKind::Immutable,
        (
            CaptureMode::Move,
            Some(CaptureTarget::Local(_)),
            Some(BindingOwnershipClass::OwnedMutable),
        ) => CaptureKind::OwnedMutable,
        (CaptureMode::Share, _, _) => CaptureKind::Shared,
        _ => {
            return Err(format!(
                "rejected declared mode `{}` reached emission",
                mode.spelling()
            ));
        }
    };
    Ok(Some(kind))
}

/// Stable diagnostic spelling for an already-selected capture kind.
///
/// Kept in the selector module so read-only query/rendering code never needs
/// to name the variants and accidentally defeat the mechanical K1 sentinel.
pub(crate) const fn capture_kind_spelling(kind: CaptureKind) -> &'static str {
    match kind {
        CaptureKind::Immutable => "immutable",
        CaptureKind::OwnedMutable => "owned-mutable",
        CaptureKind::Shared => "shared",
    }
}

/// One capture's planned outcome: the facts the selector saw, the plan it
/// produced, and — when a clause drove it — the declared mode.
#[derive(Debug, Clone)]
pub(crate) struct PlannedCapture {
    pub(crate) facts: CaptureBindingFacts,
    pub(crate) plan: CapturePlan,
    pub(crate) declared: Option<CaptureMode>,
    pub(crate) declaration_span: Option<Span>,
    pub(crate) use_spans: Vec<Span>,
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
#[path = "capture_plan/inferred_tests.rs"]
mod tests;

// ═══════════════════════════════════════════════════════════════════════════
// ADR-009 C1 SLICE 3 — THE DECLARED PATH.
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
#[path = "capture_plan/declared_tests.rs"]
mod declared_tests;
