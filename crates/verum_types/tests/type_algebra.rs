//! Boolean valuations give an independent finite model for the lattice rules.

use verum_common::List;
use verum_types::type_algebra::{
    check, prove, AtomId, BuildError, Certificate, CheckError, Expression, ExpressionDag,
    ExpressionId, Limits, Outcome, ProofId, Rule, UndeterminedReason,
};

fn proven(dag: &ExpressionDag, source: ExpressionId, target: ExpressionId) -> Certificate {
    let Outcome::Proven(certificate) = prove(dag, source, target, Limits::default()) else {
        panic!("expected a derivation for {source:?} <= {target:?}");
    };
    assert_eq!(
        check(dag, source, target, &certificate, Limits::default()),
        Ok(())
    );
    certificate
}

fn equivalent(dag: &ExpressionDag, left: ExpressionId, right: ExpressionId) {
    proven(dag, left, right);
    proven(dag, right, left);
}

#[test]
fn structural_interning_and_construction_are_bounded() {
    let mut dag = ExpressionDag::new(3);
    let a = dag.atom(AtomId(71)).unwrap();
    let b = dag.atom(AtomId(72)).unwrap();
    let union = dag.union(a, b).unwrap();
    assert_eq!(dag.atom(AtomId(71)), Ok(a));
    assert_eq!(dag.union(a, b), Ok(union));
    assert_eq!(dag.union(b, a), Err(BuildError::NodeLimit));
    assert_eq!(
        dag.intersection(a, ExpressionId(99)),
        Err(BuildError::InvalidExpression)
    );
    assert_eq!(ExpressionDag::new(0).bottom(), Err(BuildError::NodeLimit));
    assert_eq!(dag.len(), 3);
}

#[test]
fn injection_projection_and_extremes_have_the_correct_direction() {
    let mut dag = ExpressionDag::new(32);
    let bottom = dag.bottom().unwrap();
    let top = dag.top().unwrap();
    let a = dag.atom(AtomId(0)).unwrap();
    let b = dag.atom(AtomId(1)).unwrap();
    let union = dag.union(a, b).unwrap();
    let meet = dag.intersection(a, b).unwrap();
    for (source, target) in [
        (a, union),
        (b, union),
        (meet, a),
        (meet, b),
        (bottom, a),
        (a, top),
    ] {
        proven(&dag, source, target);
    }
    for (source, target) in [
        (union, a),
        (union, b),
        (a, meet),
        (b, meet),
        (a, bottom),
        (top, a),
        (a, b),
    ] {
        assert_eq!(
            prove(&dag, source, target, Limits::default()),
            Outcome::Undetermined(UndeterminedReason::NoDerivation)
        );
    }
}

#[test]
fn absorption_distributivity_and_associativity_are_derived() {
    let mut dag = ExpressionDag::new(64);
    let a = dag.atom(AtomId(0)).unwrap();
    let b = dag.atom(AtomId(1)).unwrap();
    let c = dag.atom(AtomId(2)).unwrap();
    let ab_join = dag.union(a, b).unwrap();
    let ab_meet = dag.intersection(a, b).unwrap();
    let bc_join = dag.union(b, c).unwrap();
    let bc_meet = dag.intersection(b, c).unwrap();
    let ac_join = dag.union(a, c).unwrap();
    let ac_meet = dag.intersection(a, c).unwrap();
    let absorb_join = dag.union(a, ab_meet).unwrap();
    let absorb_meet = dag.intersection(a, ab_join).unwrap();
    let distribute_meet = dag.intersection(a, bc_join).unwrap();
    let distributed_meet = dag.union(ab_meet, ac_meet).unwrap();
    let distribute_join = dag.union(a, bc_meet).unwrap();
    let distributed_join = dag.intersection(ab_join, ac_join).unwrap();
    let assoc_join_left = dag.union(ab_join, c).unwrap();
    let assoc_join_right = dag.union(a, bc_join).unwrap();
    let assoc_meet_left = dag.intersection(ab_meet, c).unwrap();
    let assoc_meet_right = dag.intersection(a, bc_meet).unwrap();
    for (left, right) in [
        (a, absorb_join),
        (a, absorb_meet),
        (distribute_meet, distributed_meet),
        (distribute_join, distributed_join),
        (assoc_join_left, assoc_join_right),
        (assoc_meet_left, assoc_meet_right),
    ] {
        equivalent(&dag, left, right);
    }
}

#[test]
fn all_small_expression_pairs_agree_with_all_boolean_valuations() {
    // Exhaust every expression with at most five tree nodes over two atoms,
    // including both constants. No search/checker helper computes the model.
    let mut dag = ExpressionDag::new(1024);
    let leaves: List<_> = [
        dag.bottom().unwrap(),
        dag.top().unwrap(),
        dag.atom(AtomId(0)).unwrap(),
        dag.atom(AtomId(1)).unwrap(),
    ]
    .into_iter()
    .collect();
    let mut three = List::new();
    for &left in &leaves {
        for &right in &leaves {
            three.push(dag.union(left, right).unwrap());
            three.push(dag.intersection(left, right).unwrap());
        }
    }
    for &short in &leaves {
        for &long in &three {
            dag.union(short, long).unwrap();
            dag.union(long, short).unwrap();
            dag.intersection(short, long).unwrap();
            dag.intersection(long, short).unwrap();
        }
    }
    assert_eq!(dag.len(), 548);
    let mut valuations: List<u8> = List::new();
    for node in dag.nodes() {
        valuations.push(match *node {
            Expression::Bottom => 0b0000,
            Expression::Top => 0b1111,
            Expression::Atom(AtomId(0)) => 0b1010,
            Expression::Atom(AtomId(1)) => 0b1100,
            Expression::Atom(_) => unreachable!(),
            Expression::Union(a, b) => valuations[a.0] | valuations[b.0],
            Expression::Intersection(a, b) => valuations[a.0] & valuations[b.0],
        });
    }
    let limits = Limits {
        max_expression_nodes: 1024,
        ..Limits::default()
    };
    let mut accepted = 0;
    for source in 0..dag.len() {
        for target in 0..dag.len() {
            let valid_in_every_model = valuations[source] & !valuations[target] == 0;
            match prove(&dag, ExpressionId(source), ExpressionId(target), limits) {
                Outcome::Proven(certificate) => {
                    assert!(valid_in_every_model, "unsound result: {source} <= {target}");
                    assert_eq!(
                        check(
                            &dag,
                            ExpressionId(source),
                            ExpressionId(target),
                            &certificate,
                            limits
                        ),
                        Ok(())
                    );
                    accepted += 1;
                }
                Outcome::Undetermined(reason) => {
                    assert_eq!(reason, UndeterminedReason::NoDerivation);
                    assert!(
                        !valid_in_every_model,
                        "missed finite entailment: {source} <= {target}"
                    );
                }
            }
        }
    }
    assert!(accepted > 10_000);
}

#[test]
fn certificates_are_bound_to_exact_expressions_and_endpoints() {
    let mut dag = ExpressionDag::new(16);
    let a = dag.atom(AtomId(7)).unwrap();
    let b = dag.atom(AtomId(8)).unwrap();
    let union = dag.union(a, b).unwrap();
    let certificate = proven(&dag, a, union);
    assert_eq!(
        check(&dag, b, union, &certificate, Limits::default()),
        Err(CheckError::QueryMismatch)
    );
    assert_eq!(
        check(&dag, a, a, &certificate, Limits::default()),
        Err(CheckError::QueryMismatch)
    );
    let mut corrupt = certificate.clone();
    corrupt.expressions[a.0] = Expression::Atom(AtomId(9));
    assert_eq!(
        check(&dag, a, union, &corrupt, Limits::default()),
        Err(CheckError::QueryMismatch)
    );
    corrupt = certificate.clone();
    corrupt.source = b;
    assert_eq!(
        check(&dag, b, union, &corrupt, Limits::default()),
        Err(CheckError::InvalidRoot)
    );
    let mut different_dag = ExpressionDag::new(16);
    different_dag.atom(AtomId(9)).unwrap();
    different_dag.atom(AtomId(8)).unwrap();
    different_dag.union(a, b).unwrap();
    assert_eq!(
        check(&different_dag, a, union, &certificate, Limits::default()),
        Err(CheckError::QueryMismatch)
    );
}

#[test]
fn checker_rejects_changed_rules_missing_branches_and_cycles() {
    let mut dag = ExpressionDag::new(16);
    let a = dag.atom(AtomId(0)).unwrap();
    let b = dag.atom(AtomId(1)).unwrap();
    let ab = dag.union(a, b).unwrap();
    let ba = dag.union(b, a).unwrap();
    let certificate = proven(&dag, ab, ba);
    let mut corrupt = certificate.clone();
    let split = corrupt
        .steps
        .iter_mut()
        .find(|step| matches!(step.rule, Rule::UnionLeft(_, _, _)))
        .unwrap();
    let Rule::UnionLeft(id, first, _) = split.rule else {
        unreachable!()
    };
    split.rule = Rule::UnionLeft(id, first, first);
    assert_eq!(
        check(&dag, ab, ba, &corrupt, Limits::default()),
        Err(CheckError::InvalidRule)
    );

    corrupt = certificate.clone();
    corrupt.steps[corrupt.root.0].rule = Rule::UnionRight(ba, corrupt.root);
    assert_eq!(
        check(&dag, ab, ba, &corrupt, Limits::default()),
        Err(CheckError::InvalidPremise)
    );

    corrupt = certificate.clone();
    corrupt.steps[0].rule = Rule::TopRight(a);
    assert_eq!(
        check(&dag, ab, ba, &corrupt, Limits::default()),
        Err(CheckError::InvalidRule)
    );

    corrupt = certificate.clone();
    corrupt.root = ProofId(0);
    assert_eq!(
        check(&dag, ab, ba, &corrupt, Limits::default()),
        Err(CheckError::InvalidRoot)
    );
}

#[test]
fn checker_requires_canonical_contexts_and_exact_subformula_scope() {
    let mut dag = ExpressionDag::new(16);
    let a = dag.atom(AtomId(0)).unwrap();
    let unrelated = dag.atom(AtomId(1)).unwrap();
    let target = dag.union(a, a).unwrap();
    let certificate = proven(&dag, a, target);
    let mut corrupt = certificate.clone();
    corrupt.steps[0].sequent.left.push(a);
    assert_eq!(
        check(&dag, a, target, &corrupt, Limits::default()),
        Err(CheckError::InvalidContext)
    );
    corrupt = certificate.clone();
    corrupt.steps[0].sequent.left.push(unrelated);
    assert_eq!(
        check(&dag, a, target, &corrupt, Limits::default()),
        Err(CheckError::InvalidExpression)
    );
    corrupt = certificate;
    corrupt.steps[0].sequent.right[0] = ExpressionId(usize::MAX);
    assert_eq!(
        check(&dag, a, target, &corrupt, Limits::default()),
        Err(CheckError::InvalidExpression)
    );
}

#[test]
fn every_resource_limit_fails_closed() {
    let mut dag = ExpressionDag::new(16);
    let a = dag.atom(AtomId(0)).unwrap();
    let target = dag.union(a, a).unwrap();
    let certificate = proven(&dag, a, target);
    for limits in [
        Limits {
            max_expression_nodes: 1,
            ..Limits::default()
        },
        Limits {
            max_sequents: 0,
            ..Limits::default()
        },
        Limits {
            max_context_entries: 1,
            ..Limits::default()
        },
        Limits {
            max_depth: 1,
            ..Limits::default()
        },
    ] {
        assert_eq!(
            prove(&dag, a, target, limits),
            Outcome::Undetermined(UndeterminedReason::ResourceLimit)
        );
        assert_eq!(
            check(&dag, a, target, &certificate, limits),
            Err(CheckError::ResourceLimit)
        );
    }
    assert_eq!(
        prove(&dag, a, ExpressionId(99), Limits::default()),
        Outcome::Undetermined(UndeterminedReason::InvalidExpression)
    );
}

#[test]
fn proof_search_is_deterministic_and_shares_duplicate_premises() {
    let mut dag = ExpressionDag::new(16);
    let a = dag.atom(AtomId(0)).unwrap();
    let duplicate = dag.union(a, a).unwrap();
    let first = proven(&dag, duplicate, a);
    assert_eq!(first.steps.len(), 2);
    let Rule::UnionLeft(_, left, right) = first.steps[first.root.0].rule else {
        panic!("expected a union split")
    };
    assert_eq!(left, right);
    assert_eq!(
        Outcome::Proven(first),
        prove(&dag, duplicate, a, Limits::default())
    );
}

#[test]
fn bounded_search_never_returns_a_certificate_over_its_replay_budget() {
    let mut dag = ExpressionDag::new(128);
    dag.bottom().unwrap();
    dag.top().unwrap();
    dag.atom(AtomId(0)).unwrap();
    dag.atom(AtomId(1)).unwrap();
    // A reproducible corpus with shared subgraphs and a mix of unary-context
    // expansion and branching. Reuse must not hide a deeper proof path.
    let mut state = 17u32;
    for index in 0..64 {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let left = ExpressionId(state as usize % dag.len());
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let right = ExpressionId(state as usize % dag.len());
        if index % 2 == 0 {
            dag.union(left, right).unwrap();
        } else {
            dag.intersection(left, right).unwrap();
        }
    }
    for depth in [1, 2, 4, 8, 16] {
        let limits = Limits {
            max_depth: depth,
            max_sequents: 32,
            max_context_entries: 128,
            ..Limits::default()
        };
        for source in 0..dag.len() {
            for target in 0..dag.len() {
                if let Outcome::Proven(certificate) =
                    prove(&dag, ExpressionId(source), ExpressionId(target), limits)
                {
                    assert_eq!(
                        check(
                            &dag,
                            ExpressionId(source),
                            ExpressionId(target),
                            &certificate,
                            limits
                        ),
                        Ok(()),
                        "accepted proof exceeds replay limits: {source} <= {target}, depth {depth}"
                    );
                }
            }
        }
    }
}

#[test]
fn deep_inputs_use_bounded_iterative_search_and_replay() {
    let mut dag = ExpressionDag::new(4096);
    let a = dag.atom(AtomId(0)).unwrap();
    let mut target = a;
    for _ in 0..2048 {
        target = dag.union(target, target).unwrap();
    }
    let limits = Limits {
        max_expression_nodes: 4096,
        max_depth: 64,
        ..Limits::default()
    };
    assert_eq!(
        prove(&dag, a, target, limits),
        Outcome::Undetermined(UndeterminedReason::ResourceLimit)
    );
    let limits = Limits {
        max_depth: 4096,
        ..limits
    };
    let Outcome::Proven(certificate) = prove(&dag, a, target, limits) else {
        panic!("expected a derivation")
    };
    assert_eq!(certificate.steps.len(), 2049);
    assert_eq!(check(&dag, a, target, &certificate, limits), Ok(()));
}
