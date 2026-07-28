use super::ConstraintSolver;
use crate::type_system::{Type, TypeVar, TypeVarGen};

fn solve_pair(left: &TypeVar, right: &TypeVar) -> ConstraintSolver {
    let mut solver = ConstraintSolver::new();
    let mut constraints = vec![(Type::Variable(left.clone()), Type::Variable(right.clone()))];
    solver
        .solve(&mut constraints)
        .expect("variable pair must unify");
    solver
}

fn assert_representative(solver: &ConstraintSolver, hole: &TypeVar, declared: &TypeVar) {
    assert_eq!(
        solver
            .unifier()
            .apply_substitutions(&Type::Variable(hole.clone())),
        Type::Variable(declared.clone())
    );
    assert!(
        solver.unifier().lookup(declared).is_none(),
        "the declared capability must remain the unbound representative"
    );
}

#[test]
fn declared_capability_represents_a_fresh_hole_in_both_orientations() {
    for declared_first in [true, false] {
        let mut variables = TypeVarGen::new();
        let declared = TypeVar::declared(variables.fresh_declared_owner(), 0, "T");
        let hole = variables.fresh_var();
        let (left, right) = if declared_first {
            (&declared, &hole)
        } else {
            (&hole, &declared)
        };

        let solver = solve_pair(left, right);
        assert_representative(&solver, &hole, &declared);
    }
}

#[test]
fn raw_variable_pairs_retain_left_to_right_binding() {
    let mut variables = TypeVarGen::new();
    let left = variables.fresh_var();
    let right = variables.fresh_var();
    let solver = solve_pair(&left, &right);

    assert_eq!(
        solver.unifier().apply_substitutions(&Type::Variable(left)),
        Type::Variable(right.clone())
    );
    assert!(solver.unifier().lookup(&right).is_none());
}

#[test]
fn declared_variable_pairs_retain_left_to_right_binding() {
    let mut variables = TypeVarGen::new();
    let left = TypeVar::declared(variables.fresh_declared_owner(), 0, "T");
    let right = TypeVar::declared(variables.fresh_declared_owner(), 0, "U");
    let solver = solve_pair(&left, &right);

    assert_eq!(
        solver.unifier().apply_substitutions(&Type::Variable(left)),
        Type::Variable(right.clone())
    );
    assert!(solver.unifier().lookup(&right).is_none());
}
