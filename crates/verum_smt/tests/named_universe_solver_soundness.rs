use verum_smt::dependent::{UniverseConstraint, UniverseConstraintSolver, UniverseLevel as Level};

#[test]
fn nested_successors_and_maxima_use_exact_assignments() {
    let u = Level::variable("u");
    let v = Level::variable("v");
    let mut solver = UniverseConstraintSolver::new();
    solver.add_eq(u.succ().succ(), Level::concrete(5));
    solver.add_eq(v.clone(), Level::max(u.succ(), Level::concrete(7)).succ());
    solver.solve().unwrap();
    assert_eq!(solver.get_assignment("u"), Some(3));
    assert_eq!(solver.get_assignment("v"), Some(8));
}

#[test]
fn names_are_interned_injectively_including_non_solver_identifiers() {
    let mut solver = UniverseConstraintSolver::new();
    for (name, value) in [("u", 3), ("u\0", 7), ("𝑢", 11), ("universe!0", 13)] {
        solver.add_eq(Level::variable(name), Level::concrete(value));
    }
    solver.solve().unwrap();
    for (name, value) in [("u", 3), ("u\0", 7), ("𝑢", 11), ("universe!0", 13)] {
        assert_eq!(solver.get_assignment(name), Some(value));
    }
    assert_eq!(solver.get_assignment("missing"), None);
}

#[test]
fn adding_a_constraint_and_failed_solving_clear_old_assignments() {
    let u = Level::variable("u");
    let mut solver = UniverseConstraintSolver::new();
    solver.add_eq(u.clone(), Level::TYPE1);
    solver.solve().unwrap();
    assert_eq!(solver.get_assignment("u"), Some(1));
    solver.add_lt(u.clone(), u);
    assert_eq!(solver.get_assignment("u"), None);
    assert!(solver.solve().is_err());
    assert_eq!(solver.get_assignment("u"), None);
}

#[test]
fn machine_overflow_remains_a_successor_and_cannot_prove_equality() {
    let boundary = Level::concrete(usize::MAX);
    let successor = boundary.succ();
    assert_ne!(successor, boundary);
    assert_eq!(successor.eval(), None);
    assert_eq!(
        UniverseConstraint::Equal(successor.clone(), Level::TYPE0).is_satisfied(),
        None
    );
    let mut solver = UniverseConstraintSolver::new();
    solver.add_eq(successor, Level::TYPE0);
    assert!(solver.solve().is_err());
}

#[test]
fn the_shared_concrete_boundary_keeps_its_successor() {
    let boundary = usize::try_from(u32::MAX).unwrap();
    let mut solver = UniverseConstraintSolver::new();
    solver.add_eq(Level::variable("u"), Level::concrete(boundary).succ());
    match boundary.checked_add(1) {
        Some(expected) => {
            solver.solve().unwrap();
            assert_eq!(solver.get_assignment("u"), Some(expected));
        }
        None => {
            assert!(solver.solve().is_err());
            assert_eq!(solver.get_assignment("u"), None);
        }
    }
}

#[test]
fn resource_refusal_cannot_publish_a_partial_model() {
    let mut solver = UniverseConstraintSolver::new();
    for _ in 0..=verum_types::universe_solver::SolverLimits::default().max_constraints {
        solver.add_eq(Level::variable("u"), Level::TYPE1);
    }
    assert!(solver.solve().is_err());
    assert_eq!(solver.get_assignment("u"), None);
}
