//! The R21 named tracer (issue #178, ADR-014 §8).
//!
//! The tracer program is
//!
//! ```shape
//! fn apply<T, effect F>(f: fn() -> T ! F) -> T ! F { return f() }
//! ```
//!
//! and the three cases it has to carry are the acceptance criteria verbatim:
//! positive subsumption, a negative boundary whose diagnostic carries the
//! inferred row, and instantiation closure — no unbound effect parameter
//! surviving into a persisted fact.
//!
//! These run against the real `ConstraintSolver`, not a stand-in: the
//! subsumption decision, the binder closure, and the structured error all
//! come from the same code paths ordinary checking uses. The tracer's SOURCE
//! is proved to parse to these rows separately, in
//! `shape_ast::parser::tests::effect_rows::effect_binders_parse_in_a_generic_parameter_list`;
//! together the two ends meet.

#[cfg(test)]
mod tests {
    use crate::type_system::constraints::ConstraintSolver;
    use crate::type_system::effects::{
        ClosedEffectRow, EffectAtom, EffectParamRef, EffectRow, EffectStage, OperationalEffectId,
    };
    use crate::type_system::errors::TypeError;
    use crate::type_system::types::{BuiltinTypes, Type};

    const FS_READ: EffectAtom = EffectAtom::Operation(OperationalEffectId::FsRead);
    const NET_CONNECT: EffectAtom = EffectAtom::Operation(OperationalEffectId::NetConnect);

    fn closed(atoms: &[EffectAtom]) -> EffectRow {
        EffectRow::Closed(
            ClosedEffectRow::from_atoms(EffectStage::Runtime, atoms.iter().copied()).unwrap(),
        )
    }

    fn pure_row() -> EffectRow {
        EffectRow::pure(EffectStage::Runtime)
    }

    /// `fn() -> int ! <row>` — the callback type `apply` takes.
    fn callback(row: EffectRow) -> Type {
        Type::function_with_effects(vec![], BuiltinTypes::number(), row)
    }

    /// The declared type of `apply` at a given instantiation of `F`:
    /// `fn(fn() -> int ! F) -> int ! F`.
    fn apply_signature(row: EffectRow) -> Type {
        Type::function_with_effects(vec![callback(row.clone())], BuiltinTypes::number(), row)
    }

    // ---------------------------------------------------------------------
    // (a) Positive subsumption
    // ---------------------------------------------------------------------

    #[test]
    fn tracer_a_a_pure_closure_is_accepted_where_fs_read_is_declared() {
        let mut solver = ConstraintSolver::new();
        let declared = callback(closed(&[FS_READ]));
        let actual = callback(pure_row());

        solver
            .check_declared_boundary(&actual, &declared)
            .expect("a pure closure must be usable where `! {FsRead}` is accepted");
    }

    #[test]
    fn tracer_a_subsumption_is_subset_and_not_equality() {
        // The same check must reject the reverse direction, or "accepted"
        // above would just mean "rows are ignored".
        let mut solver = ConstraintSolver::new();
        let declared = callback(pure_row());
        let effectful = callback(closed(&[FS_READ]));

        assert!(
            solver
                .check_declared_boundary(&effectful, &declared)
                .is_err(),
            "subset is directional: `{{FsRead}}` must NOT fit `! {{}}`"
        );
    }

    #[test]
    fn tracer_a_a_narrower_row_fits_a_wider_one() {
        let mut solver = ConstraintSolver::new();
        let declared = callback(closed(&[FS_READ, NET_CONNECT]));
        let actual = callback(closed(&[FS_READ]));
        solver
            .check_declared_boundary(&actual, &declared)
            .expect("{FsRead} is a subset of {FsRead, NetConnect}");
    }

    // ---------------------------------------------------------------------
    // (b) Negative boundary — the diagnostic carries the inferred row
    // ---------------------------------------------------------------------

    #[test]
    fn tracer_b_a_closure_exceeding_its_boundary_rejects_with_the_inferred_row() {
        let mut solver = ConstraintSolver::new();
        let declared = callback(closed(&[FS_READ]));
        let inferred = callback(closed(&[FS_READ, NET_CONNECT]));

        let error = solver
            .check_declared_boundary(&inferred, &declared)
            .expect_err("a closure whose row exceeds the boundary must reject");

        match error {
            TypeError::EffectRowExceedsBoundary {
                inferred,
                declared,
                excess,
            } => {
                // The payload #180's materialization fix consumes: what the
                // interior actually does, what the boundary allows, and the
                // exact atoms to add. Not a bare "type mismatch".
                assert_eq!(inferred, "{FsRead, NetConnect}");
                assert_eq!(declared, "{FsRead}");
                assert_eq!(excess, vec!["NetConnect".to_string()]);
            }
            other => panic!("expected a structured effect-row diagnostic, got {other:?}"),
        }
    }

    #[test]
    fn tracer_b_the_excess_list_is_sorted_regardless_of_insertion_order() {
        // #205: nothing that reaches a diagnostic may depend on iteration
        // order. Build the same excess two ways and demand one rendering.
        let declared = callback(pure_row());
        let forward = callback(closed(&[FS_READ, NET_CONNECT]));
        let backward = callback(closed(&[NET_CONNECT, FS_READ]));

        let render = |ty: &Type| {
            let mut solver = ConstraintSolver::new();
            match solver.check_declared_boundary(ty, &declared).unwrap_err() {
                TypeError::EffectRowExceedsBoundary {
                    inferred, excess, ..
                } => (inferred, excess),
                other => panic!("unexpected {other:?}"),
            }
        };
        assert_eq!(render(&forward), render(&backward));
        assert_eq!(
            render(&forward),
            (
                "{FsRead, NetConnect}".to_string(),
                vec!["FsRead".to_string(), "NetConnect".to_string()]
            )
        );
    }

    #[test]
    fn tracer_b_a_nested_callback_row_is_checked_contravariantly() {
        // ADR-014 §8.1's nesting rule: `fn(fn() -> T ! E1) -> U ! E2` accepts
        // an argument typed `fn(fn() -> T ! E3) -> U ! E4` iff `E1 ⊆ E3` and
        // `E4 ⊆ E2`. Variance flips at the parameter, so a candidate that
        // accepts a WIDER callback is more permissive and is fine, while one
        // that accepts only a NARROWER callback is not.
        let declared = apply_signature(closed(&[FS_READ]));

        let accepts_wider = Type::function_with_effects(
            vec![callback(closed(&[FS_READ, NET_CONNECT]))],
            BuiltinTypes::number(),
            closed(&[FS_READ]),
        );
        ConstraintSolver::new()
            .check_declared_boundary(&accepts_wider, &declared)
            .expect("E1 ⊆ E3 holds: accepting a wider callback is safe");

        let accepts_narrower = Type::function_with_effects(
            vec![callback(pure_row())],
            BuiltinTypes::number(),
            closed(&[FS_READ]),
        );
        assert!(
            ConstraintSolver::new()
                .check_declared_boundary(&accepts_narrower, &declared)
                .is_err(),
            "E1 ⊆ E3 fails: a candidate accepting only pure callbacks cannot \
             stand where `! {{FsRead}}` callbacks are passed"
        );
    }

    // ---------------------------------------------------------------------
    // (c) Instantiation closure
    // ---------------------------------------------------------------------

    #[test]
    fn tracer_c_instantiating_the_binder_closes_it_to_a_closed_row() {
        let mut solver = ConstraintSolver::new();
        // `apply<T, effect F>` before instantiation: the parameter row is the
        // binder `F`.
        let generic = callback(EffectRow::param("F"));
        // The call site passes a closure with a concrete row.
        let argument = callback(closed(&[FS_READ]));

        solver
            .check_declared_boundary(&argument, &generic)
            .expect("a closed row instantiates the binder");

        let subst = solver.effect_substitution();
        let bound = subst
            .get(&EffectParamRef::new("F"))
            .expect("`F` must be bound by the instantiation");
        assert_eq!(bound.render(), "{FsRead}");
    }

    #[test]
    fn tracer_c_an_unsubstituted_binder_yields_no_closed_evidence() {
        // The mechanical half: `prove_closed` is the ONLY route to a closed
        // row, and its failure type cannot be constructed outside the effects
        // module, so no checking code can fabricate the proof.
        let unbound = EffectRow::param("F");
        assert!(unbound.prove_closed().is_err());
        assert!(!unbound.is_persistable_as_fact());
        // A generic SCHEMA may still publish the binder (§8.3).
        assert!(unbound.is_persistable_in_schema());
    }

    #[test]
    fn tracer_c_a_binder_used_at_two_rows_closes_to_their_union() {
        // One instantiation, two call-through-value sites. §3's join is
        // canonical union, so the binder closes to the least row satisfying
        // both — never to whichever site was visited last.
        let mut solver = ConstraintSolver::new();
        let generic = callback(EffectRow::param("F"));

        solver
            .check_declared_boundary(&callback(closed(&[FS_READ])), &generic)
            .unwrap();
        solver
            .check_declared_boundary(&callback(closed(&[NET_CONNECT])), &generic)
            .unwrap();

        assert_eq!(
            solver
                .effect_substitution()
                .get(&EffectParamRef::new("F"))
                .unwrap()
                .render(),
            "{FsRead, NetConnect}"
        );
    }

    #[test]
    fn tracer_c_a_binder_cannot_be_proved_against_a_closed_boundary() {
        // The direction that must NOT work: an unsubstituted binder standing
        // where a closed row is required. ADR-010 §13 — it had to close
        // first, so this is a diagnostic and not a silent pass.
        let mut solver = ConstraintSolver::new();
        let error = solver
            .check_declared_boundary(
                &callback(EffectRow::param("F")),
                &callback(closed(&[FS_READ])),
            )
            .expect_err("an unbound binder must not satisfy a closed boundary");
        match error {
            TypeError::UnboundEffectParameter { parameter, .. } => assert_eq!(parameter, "F"),
            other => panic!("expected an unbound-parameter diagnostic, got {other:?}"),
        }
    }

    // ---------------------------------------------------------------------
    // Caveat two: closures carry per-VALUE rows
    // ---------------------------------------------------------------------

    #[test]
    fn caveat_two_two_closures_in_one_scope_carry_different_rows() {
        // Before this work a closure had no permission identity at all — its
        // body stamped the ENCLOSING blob, so every closure in a function was
        // indistinguishable. §8.1 requires a per-closure-value row, and at
        // the type level that means two closure values in one scope are
        // different TYPES when their rows differ.
        let mut solver = ConstraintSolver::new();
        let logging = callback(closed(&[FS_READ]));
        let quiet = callback(pure_row());

        assert!(
            !solver.probe_equal(&logging, &quiet),
            "closures differing only in row must not be the same type"
        );
        // And the asymmetry is the subset order, not mere inequality.
        assert!(solver.check_declared_boundary(&quiet, &logging).is_ok());
        assert!(solver.check_declared_boundary(&logging, &quiet).is_err());
    }

    #[test]
    fn caveat_two_the_row_survives_being_carried_through_the_type() {
        // The row is reachable from the type itself, so any consumer — the
        // checker, the LSP projection, the contract publisher — reads the
        // same fact rather than re-deriving one.
        let ty = callback(closed(&[FS_READ]));
        assert_eq!(
            ty.effect_row().expect("a function type has a row").render(),
            "{FsRead}"
        );
        assert!(BuiltinTypes::number().effect_row().is_none());
    }
}
