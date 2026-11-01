use verum_common::{List, Map};
use verum_types::{
    context::{UniverseConstraint as Constraint, UniverseContext, UniverseSubstitution},
    ty::UniverseLevel as Level,
    universe_solver::{
        SolverLimits, solve_universe_constraints, solve_with_limits, validate_assignment,
    },
};

fn var(id: u32) -> Level {
    Level::Variable(id)
}
fn concrete(value: u32) -> Level {
    Level::Concrete(value)
}
fn succ(level: Level) -> Level {
    Level::Succ(Box::new(level))
}
fn max(left: Level, right: Level) -> Level {
    Level::Max(Box::new(left), Box::new(right))
}
fn value(substitution: &UniverseSubstitution, id: u32) -> u64 {
    substitution
        .resolve(&var(id))
        .unwrap()
        .evaluate(&Map::new())
        .unwrap()
}

#[test]
fn unresolved_constraints_are_not_model_evidence() {
    let empty = UniverseSubstitution::new();
    for constraint in [
        Constraint::LessOrEqual(var(0), var(1)),
        Constraint::StrictlyLess(var(0), var(1)),
        Constraint::Equal(var(0), var(0)),
        Constraint::Max(concrete(2), var(0), concrete(1)),
        Constraint::Successor(var(0), var(1)),
    ] {
        assert!(!constraint.is_satisfied(&empty));
    }
    assert!(Constraint::LessOrEqual(concrete(0), concrete(1)).is_satisfied(&empty));
}

#[test]
fn conflicting_bounds_and_strict_cycles_are_rejected() {
    let cases = [
        vec![
            Constraint::LessOrEqual(concrete(5), var(0)),
            Constraint::LessOrEqual(var(0), concrete(3)),
        ],
        vec![
            Constraint::Equal(var(0), concrete(1)),
            Constraint::Equal(var(0), concrete(2)),
        ],
        vec![
            Constraint::StrictlyLess(var(0), var(1)),
            Constraint::StrictlyLess(var(1), var(0)),
        ],
        vec![Constraint::Successor(var(0), var(0))],
        vec![Constraint::Max(concrete(0), var(0), concrete(1))],
    ];
    for mut constraints in cases {
        assert!(solve_universe_constraints(&constraints).is_err());
        constraints.reverse();
        assert!(solve_universe_constraints(&constraints).is_err());
    }
}

#[test]
fn constraint_order_does_not_change_model_validity() {
    let constraints = [
        Constraint::LessOrEqual(concrete(3), var(0)),
        Constraint::LessOrEqual(var(0), concrete(5)),
        Constraint::Successor(var(1), var(0)),
        Constraint::Max(var(2), var(1), concrete(10)),
        Constraint::Equal(var(3), max(var(2), succ(var(0)))),
    ];
    for offset in 0..constraints.len() {
        let reordered: List<_> = constraints
            .iter()
            .cycle()
            .skip(offset)
            .take(constraints.len())
            .cloned()
            .collect();
        let solution = solve_universe_constraints(&reordered).unwrap();
        assert!(
            constraints
                .iter()
                .all(|constraint| constraint.is_satisfied(&solution))
        );
        assert!((3..=5).contains(&value(&solution, 0)));
        assert_eq!(value(&solution, 1), value(&solution, 0) + 1);
        assert_eq!(value(&solution, 2), 10);
        assert_eq!(value(&solution, 3), 10);
    }
}

#[test]
fn long_successor_chains_are_solved_and_original_constraints_rechecked() {
    let mut constraints = List::from_single(Constraint::Equal(var(0), concrete(0)));
    for id in 1..=256 {
        constraints.push(Constraint::Successor(var(id), var(id - 1)));
    }
    constraints.reverse();
    let solution = solve_universe_constraints(&constraints).unwrap();
    assert_eq!(value(&solution, 256), 256);
    assert!(
        constraints
            .iter()
            .all(|constraint| constraint.is_satisfied(&solution))
    );
}

#[test]
fn nested_maxima_and_successors_keep_every_operand() {
    let expression = max(succ(var(7)), max(concrete(4), succ(succ(var(8)))));
    let constraints = [
        Constraint::Equal(var(7), concrete(5)),
        Constraint::Equal(var(8), concrete(2)),
        Constraint::Equal(var(9), expression),
    ];
    let solution = solve_universe_constraints(&constraints).unwrap();
    assert_eq!(value(&solution, 9), 6);
    assert!(
        constraints
            .iter()
            .all(|constraint| constraint.is_satisfied(&solution))
    );
}

#[test]
fn successor_of_the_largest_concrete_level_is_preserved() {
    let constraints = [Constraint::Successor(var(0), concrete(u32::MAX))];
    let solution = solve_universe_constraints(&constraints).unwrap();
    assert_eq!(value(&solution, 0), u64::from(u32::MAX) + 1);
    assert!(constraints[0].is_satisfied(&solution));
    assert!(
        solve_universe_constraints(&[Constraint::Equal(
            succ(concrete(u32::MAX)),
            concrete(u32::MAX)
        )])
        .is_err()
    );
    let opaque_id =
        solve_universe_constraints(&[Constraint::Equal(var(u32::MAX), concrete(1))]).unwrap();
    assert_eq!(value(&opaque_id, u32::MAX), 1);
}

#[test]
fn insertion_rejects_cycles_transactionally() {
    let mut substitution = UniverseSubstitution::new();
    assert!(substitution.insert(0, var(0)).is_err());
    assert_eq!(substitution.resolve(&var(0)).unwrap(), var(0));
    substitution.insert(0, var(1)).unwrap();
    assert!(substitution.insert(1, succ(var(0))).is_err());
    assert_eq!(substitution.resolve(&var(0)).unwrap(), var(1));
    substitution.insert(1, concrete(4)).unwrap();
    assert_eq!(value(&substitution, 0), 4);
    assert!(substitution.insert(1, max(var(0), concrete(0))).is_err());
    assert_eq!(value(&substitution, 0), 4);
}

#[test]
fn merging_cannot_install_a_cycle_or_partially_replace_bindings() {
    let mut left = UniverseSubstitution::new();
    left.insert(0, var(1)).unwrap();
    left.insert(2, concrete(3)).unwrap();
    let mut right = UniverseSubstitution::new();
    right.insert(1, var(0)).unwrap();
    right.insert(2, concrete(8)).unwrap();
    assert!(left.merge(&right).is_err());
    assert_eq!(left.resolve(&var(0)).unwrap(), var(1));
    assert_eq!(value(&left, 2), 3);
}

#[test]
fn alias_resolution_is_iterative_and_bounded() {
    let mut substitution = UniverseSubstitution::new();
    for id in 0..1024 {
        substitution.insert(id, var(id + 1)).unwrap();
    }
    substitution.insert(1024, concrete(7)).unwrap();
    assert_eq!(value(&substitution, 0), 7);
    for id in 1024..4200 {
        substitution.insert(id, var(id + 1)).unwrap();
    }
    assert!(substitution.resolve(&var(0)).is_err());
}

#[test]
fn independent_model_validation_rejects_missing_wrong_and_overflowing_values() {
    let constraint = Constraint::Successor(var(0), var(1));
    let mut assignment = Map::new();
    assert!(validate_assignment(&[constraint.clone()], &assignment).is_err());
    assignment.insert(0, 0);
    assignment.insert(1, u64::MAX);
    assert!(validate_assignment(&[constraint.clone()], &assignment).is_err());
    assignment.insert(1, 0);
    assert!(validate_assignment(&[constraint.clone()], &assignment).is_err());
    assignment.insert(0, 1);
    let accepted = validate_assignment(&[constraint], &assignment).unwrap();
    assert_eq!(value(&accepted, 0), 1);
    assignment.insert(2, u64::MAX);
    assert!(validate_assignment(&[], &assignment).is_err());
}

#[test]
fn added_constraints_invalidate_old_solutions_even_when_the_new_solve_fails() {
    let mut context = UniverseContext::new();
    let subject = context.fresh_universe_var().unwrap();
    let initial = Constraint::Equal(subject.clone(), concrete(1));
    context.add_constraint(initial.clone());
    context.solve().unwrap();
    assert!(initial.is_satisfied(context.substitution()));
    context.add_constraint(Constraint::Equal(subject.clone(), concrete(2)));
    assert!(!initial.is_satisfied(context.substitution()));
    assert!(context.solve().is_err());
    assert!(!initial.is_satisfied(context.substitution()));
    assert_eq!(context.resolve(&subject).unwrap(), subject);
}

#[test]
fn context_maximum_constraints_support_empty_and_multiple_operands() {
    let mut context = UniverseContext::new();
    let empty = context.fresh_universe_var().unwrap();
    let maximum = context.fresh_universe_var().unwrap();
    context.add_max_constraint(empty.clone(), List::new());
    context.add_max_constraint(
        maximum.clone(),
        vec![concrete(2), concrete(9), concrete(4)].into(),
    );
    context.solve().unwrap();
    assert_eq!(context.resolve(&empty).unwrap(), concrete(0));
    assert_eq!(context.resolve(&maximum).unwrap(), concrete(9));
}

#[test]
fn context_reserves_explicit_ids_without_treating_their_magnitude_as_a_count() {
    let mut context = UniverseContext::new();
    context.add_constraint(Constraint::Equal(var(0), concrete(5)));
    context.add_constraint(Constraint::Equal(var(u32::MAX), concrete(7)));
    let fresh = context.fresh_universe_var().unwrap();
    assert_eq!(fresh, var(1));
    let next = context.fresh_universe_var().unwrap();
    assert_eq!(next, var(2));
    context.solve().unwrap();
    assert_eq!(context.resolve(&var(0)).unwrap(), concrete(5));
    assert_eq!(context.resolve(&var(u32::MAX)).unwrap(), concrete(7));
    assert_eq!(context.resolve(&fresh).unwrap(), concrete(0));
    assert_eq!(context.resolve(&next).unwrap(), concrete(0));
}

#[test]
fn allocator_budget_counts_reserved_variables_and_returns_an_error() {
    let mut context = UniverseContext::new();
    for id in 0..SolverLimits::default().max_variables as u32 {
        context.add_constraint(Constraint::Equal(var(100_000 + id), concrete(0)));
    }
    assert!(context.fresh_universe_var().is_err());
}

#[test]
fn public_model_validation_enforces_collection_and_representation_budgets() {
    let limits = SolverLimits::default();
    let constraints: List<_> = (0..=limits.max_constraints)
        .map(|_| Constraint::Equal(concrete(0), concrete(0)))
        .collect();
    assert!(validate_assignment(&constraints, &Map::new()).is_err());
    let oversized_assignment: Map<_, _> = (0..=limits.max_variables as u32)
        .map(|id| (id, 0))
        .collect();
    assert!(validate_assignment(&[], &oversized_assignment).is_err());
    let oversized_representation: Map<_, _> =
        (0..256).map(|id| (id, u64::from(u32::MAX) + 512)).collect();
    assert!(validate_assignment(&[], &oversized_representation).is_err());
    let mut expression = concrete(0);
    for _ in 0..4096 {
        expression = succ(expression);
    }
    assert!(
        validate_assignment(
            &[Constraint::Equal(expression, concrete(4096))],
            &Map::new()
        )
        .is_err()
    );
}

#[test]
fn search_limits_and_oversized_expressions_never_produce_success() {
    let constraint = Constraint::Equal(var(0), concrete(1));
    for limits in [
        SolverLimits {
            max_constraints: 0,
            ..SolverLimits::default()
        },
        SolverLimits {
            max_expression_nodes: 0,
            ..SolverLimits::default()
        },
        SolverLimits {
            max_variables: 0,
            ..SolverLimits::default()
        },
        SolverLimits {
            solver_steps: 0,
            ..SolverLimits::default()
        },
    ] {
        assert!(solve_with_limits(&[constraint.clone()], limits).is_err());
    }
    let mut oversized = concrete(0);
    for _ in 0..4096 {
        oversized = succ(oversized);
    }
    assert!(solve_universe_constraints(&[Constraint::Equal(oversized, concrete(0))]).is_err());
}
